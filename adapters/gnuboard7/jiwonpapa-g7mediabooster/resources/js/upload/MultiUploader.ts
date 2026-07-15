import type {
    CompletedPart,
    DirectUploadTransport,
    FileUploadProgress,
    FileUploadResult,
    MediaControlClient,
    UploadBatchResult,
    UploadFileIntent,
    UploadIntent,
} from '../types';
import { Semaphore } from './Semaphore';
import { UploadTransportError } from './XhrUploadTransport';

export interface MultiUploaderOptions {
    maxParallelFiles?: number;
    maxParallelParts?: number;
    maxConnections?: number;
    maxRetries?: number;
    signal?: AbortSignal;
    onProgress?: (progress: FileUploadProgress) => void;
}

interface PreparedFile {
    file: File;
    intent: UploadFileIntent;
}

export class MultiUploader {
    public constructor(
        private readonly control: MediaControlClient,
        private readonly transport: DirectUploadTransport,
    ) {}

    public async upload(files: Iterable<File>, options: MultiUploaderOptions = {}): Promise<UploadBatchResult> {
        const prepared = prepareFiles(files);
        if (prepared.length === 0 || prepared.length > 100) {
            throw new RangeError('a multi-upload batch must contain 1-100 files');
        }

        const maxParallelFiles = boundedOption(options.maxParallelFiles, 8, 1, 16, 'maxParallelFiles');
        const maxParallelParts = boundedOption(options.maxParallelParts, 4, 1, 8, 'maxParallelParts');
        const maxConnections = boundedOption(options.maxConnections, 8, 1, 16, 'maxConnections');
        const maxRetries = boundedOption(options.maxRetries, 3, 0, 5, 'maxRetries');
        const signal = options.signal ?? new AbortController().signal;
        signal.throwIfAborted();

        const batch = await this.control.createBatch(prepared.map(({ intent }) => intent));
        const instructions = validateBatch(batch.uploads, prepared);
        const connections = new Semaphore(maxConnections);
        const results = await mapLimit(
            prepared,
            maxParallelFiles,
            async ({ file, intent }): Promise<FileUploadResult> => {
                const instruction = instructions.get(intent.client_ref);
                if (!instruction) {
                    throw new Error('validated upload instruction is missing');
                }
                return this.uploadOne(
                    file,
                    intent.client_ref,
                    instruction,
                    signal,
                    connections,
                    maxParallelParts,
                    maxRetries,
                    options.onProgress,
                );
            },
        );

        return { batchId: batch.batch_id, files: results };
    }

    private async uploadOne(
        file: File,
        clientRef: string,
        instruction: UploadIntent,
        signal: AbortSignal,
        connections: Semaphore,
        maxParallelParts: number,
        maxRetries: number,
        onProgress?: (progress: FileUploadProgress) => void,
    ): Promise<FileUploadResult> {
        const emit = (state: FileUploadProgress['state'], bytesSent: number, error?: string): void => {
            onProgress?.({
                clientRef,
                file,
                uploadId: instruction.upload_id,
                state,
                bytesSent,
                totalBytes: file.size,
                percent: file.size === 0 ? 0 : Math.floor((bytesSent / file.size) * 100),
                ...(error ? { error } : {}),
            });
        };
        emit('queued', 0);

        try {
            signal.throwIfAborted();
            if (instruction.method === 'single_put') {
                await this.uploadSingle(file, instruction, signal, connections, maxRetries, (loaded) => emit('uploading', loaded));
            } else {
                await this.uploadMultipart(
                    file,
                    instruction,
                    signal,
                    connections,
                    maxParallelParts,
                    maxRetries,
                    (loaded) => emit('uploading', loaded),
                );
            }
            emit('accepted', file.size);
            return { clientRef, uploadId: instruction.upload_id, file, state: 'accepted' };
        } catch (error) {
            const cancelled = isAbortError(error) || signal.aborted;
            const message = cancelled ? '업로드가 취소되었습니다.' : safeErrorMessage(error);
            emit(cancelled ? 'cancelled' : 'failed', 0, message);
            return {
                clientRef,
                uploadId: instruction.upload_id,
                file,
                state: cancelled ? 'cancelled' : 'failed',
                error: message,
            };
        }
    }

    private async uploadSingle(
        file: File,
        instruction: UploadIntent,
        signal: AbortSignal,
        connections: Semaphore,
        maxRetries: number,
        onProgress: (loaded: number) => void,
    ): Promise<void> {
        if (!instruction.upload_url) {
            throw new Error('single PUT instruction has no upload URL');
        }
        await retry(
            maxRetries,
            signal,
            async () => connections.use(signal, () => this.transport.put(
                instruction.upload_url as string,
                file,
                instruction.required_headers,
                signal,
                onProgress,
            )),
        );
        await this.control.confirmSingle(instruction.upload_id);
    }

    private async uploadMultipart(
        file: File,
        instruction: UploadIntent,
        outerSignal: AbortSignal,
        connections: Semaphore,
        maxParallelParts: number,
        maxRetries: number,
        onProgress: (loaded: number) => void,
    ): Promise<void> {
        const partSize = instruction.part_size_bytes;
        if (!Number.isSafeInteger(partSize) || (partSize ?? 0) < 5 * 1024 * 1024) {
            throw new Error('multipart instruction has an invalid part size');
        }
        const normalizedPartSize = partSize as number;
        const partCount = Math.ceil(file.size / normalizedPartSize);
        if (partCount < 1 || partCount > 10_000) {
            throw new Error('multipart part count is outside the provider limit');
        }

        const controller = new AbortController();
        const abort = (): void => controller.abort(outerSignal.reason);
        outerSignal.addEventListener('abort', abort, { once: true });
        const loadedByPart = new Map<number, number>();
        const updateProgress = (partNumber: number, loaded: number): void => {
            loadedByPart.set(partNumber, loaded);
            onProgress([...loadedByPart.values()].reduce((sum, value) => sum + value, 0));
        };

        try {
            const partNumbers = Array.from({ length: partCount }, (_, index) => index + 1);
            const completed = await mapLimit(partNumbers, maxParallelParts, async (partNumber): Promise<CompletedPart> => {
                const start = (partNumber - 1) * normalizedPartSize;
                const end = Math.min(file.size, start + normalizedPartSize);
                const blob = file.slice(start, end);
                try {
                    return await retry(maxRetries, controller.signal, async () => {
                        updateProgress(partNumber, 0);
                        const signed = await this.control.presignPart(instruction.upload_id, partNumber, blob.size);
                        const response = await connections.use(controller.signal, () => this.transport.put(
                            signed.upload_url,
                            blob,
                            signed.required_headers,
                            controller.signal,
                            (loaded) => updateProgress(partNumber, loaded),
                        ));
                        const etag = response.etag?.trim() ?? '';
                        if (!etag || etag.length > 1024 || !/^[\x21-\x7e]+$/.test(etag)) {
                            throw new Error('object-store CORS must expose a valid ETag header');
                        }
                        return { part_number: partNumber, etag };
                    });
                } catch (error) {
                    controller.abort(error);
                    throw error;
                }
            });
            await this.control.completeMultipart(instruction.upload_id, completed);
        } catch (error) {
            try {
                await this.control.abortMultipart(instruction.upload_id);
            } catch {
                // Lifecycle cleanup is also enforced by the object-store abort policy.
            }
            throw error;
        } finally {
            outerSignal.removeEventListener('abort', abort);
        }
    }
}

function prepareFiles(files: Iterable<File>): PreparedFile[] {
    return Array.from(files, (file): PreparedFile => {
        if (!(file instanceof File) || file.size <= 0 || !Number.isSafeInteger(file.size)) {
            throw new TypeError('all upload entries must be non-empty browser File objects');
        }
        const declaredKind = classifyFile(file);
        const contentType = normalizeContentType(file.type, declaredKind);
        return {
            file,
            intent: {
                client_ref: createClientRef(),
                declared_kind: declaredKind,
                content_length: file.size,
                content_type_hint: contentType,
            },
        };
    });
}

function classifyFile(file: File): 'image' | 'video' {
    if (file.type.startsWith('image/')) return 'image';
    if (file.type.startsWith('video/')) return 'video';
    const extension = file.name.split('.').pop()?.toLowerCase() ?? '';
    if (['jpg', 'jpeg', 'png', 'gif', 'webp', 'avif', 'heic', 'heif'].includes(extension)) return 'image';
    if (['mp4', 'mov', 'webm'].includes(extension)) return 'video';
    throw new TypeError(`unsupported media type: ${file.name}`);
}

function normalizeContentType(type: string, kind: 'image' | 'video'): string {
    const normalized = type.trim().toLowerCase();
    if (normalized && /^[\x21-\x7e]{1,255}$/.test(normalized)) {
        return normalized;
    }
    return kind === 'image' || kind === 'video' ? 'application/octet-stream' : normalized;
}

function createClientRef(): string {
    const random = typeof crypto.randomUUID === 'function'
        ? crypto.randomUUID().replaceAll('-', '')
        : Array.from(crypto.getRandomValues(new Uint8Array(16)), (byte) => byte.toString(16).padStart(2, '0')).join('');
    return `f_${random}`;
}

function validateBatch(instructions: UploadIntent[], files: PreparedFile[]): Map<string, UploadIntent> {
    if (!Array.isArray(instructions) || instructions.length !== files.length) {
        throw new Error('control API returned an incomplete upload batch');
    }
    const expected = new Set(files.map(({ intent }) => intent.client_ref));
    const result = new Map<string, UploadIntent>();
    for (const instruction of instructions) {
        if (!expected.has(instruction.client_ref) || result.has(instruction.client_ref)) {
            throw new Error('control API returned an invalid client_ref mapping');
        }
        if (!['single_put', 'multipart'].includes(instruction.method)) {
            throw new Error('control API returned an unsupported upload method');
        }
        result.set(instruction.client_ref, instruction);
    }
    return result;
}

async function mapLimit<T, R>(items: readonly T[], limit: number, mapper: (item: T, index: number) => Promise<R>): Promise<R[]> {
    const results = new Array<R>(items.length);
    let next = 0;
    let firstError: unknown;
    const worker = async (): Promise<void> => {
        while (firstError === undefined) {
            const index = next;
            next += 1;
            if (index >= items.length) return;
            const item = items[index];
            if (item === undefined) return;
            try {
                results[index] = await mapper(item, index);
            } catch (error) {
                firstError = error;
            }
        }
    };
    await Promise.allSettled(Array.from({ length: Math.min(limit, items.length) }, worker));
    if (firstError !== undefined) throw firstError;
    return results;
}

async function retry<T>(maxRetries: number, signal: AbortSignal, operation: () => Promise<T>): Promise<T> {
    let attempt = 0;
    while (true) {
        signal.throwIfAborted();
        try {
            return await operation();
        } catch (error) {
            if (attempt >= maxRetries || !isTransient(error)) throw error;
            attempt += 1;
            await abortableDelay(Math.min(250 * 2 ** (attempt - 1), 2_000), signal);
        }
    }
}

function isTransient(error: unknown): boolean {
    if (error instanceof UploadTransportError) return error.transient;
    const status = (error as { response?: { status?: unknown } } | null)?.response?.status;
    return typeof status === 'number' && (status === 408 || status === 429 || status >= 500);
}

function abortableDelay(milliseconds: number, signal: AbortSignal): Promise<void> {
    return new Promise((resolve, reject) => {
        const timer = window.setTimeout(() => {
            signal.removeEventListener('abort', abort);
            resolve();
        }, milliseconds);
        const abort = (): void => {
            window.clearTimeout(timer);
            reject(signal.reason ?? new DOMException('Upload aborted', 'AbortError'));
        };
        signal.addEventListener('abort', abort, { once: true });
    });
}

function boundedOption(value: number | undefined, fallback: number, min: number, max: number, name: string): number {
    const resolved = value ?? fallback;
    if (!Number.isInteger(resolved) || resolved < min || resolved > max) {
        throw new RangeError(`${name} must be between ${min} and ${max}`);
    }
    return resolved;
}

function isAbortError(error: unknown): boolean {
    return error instanceof DOMException && error.name === 'AbortError';
}

function safeErrorMessage(error: unknown): string {
    if (error instanceof Error && error.message && error.message.length <= 240) return error.message;
    return '업로드를 완료하지 못했습니다.';
}
