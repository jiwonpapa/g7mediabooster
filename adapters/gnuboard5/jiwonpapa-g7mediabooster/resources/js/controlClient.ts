import type {
    CompletedPart,
    MediaControlClient,
    NativeAttachment,
    PresignedPart,
    PublicUploaderConfiguration,
    UploadBatch,
    UploadFileIntent,
    UploadStatus,
} from '../../../../gnuboard7/jiwonpapa-g7mediabooster/resources/js/types';

export interface G5BrowserConfiguration {
    apiUrl: string;
    assetUrl: string;
    boardTable: string;
    csrfToken: string;
    version: string;
}

interface Envelope<T> {
    success: boolean;
    data?: T;
    message?: string;
}

export class G5MediaControlClient implements MediaControlClient {
    public constructor(
        private readonly browserConfig: G5BrowserConfiguration = requiredConfiguration(),
        private readonly fetcher: typeof fetch = globalThis.fetch.bind(globalThis),
    ) {
        if (!isSafeConfiguration(browserConfig)) {
            throw new TypeError('invalid G5 MediaBooster browser configuration');
        }
    }

    public async configuration(): Promise<PublicUploaderConfiguration> {
        return this.request('configuration', 'GET');
    }

    public async createBatch(files: UploadFileIntent[]): Promise<UploadBatch> {
        return this.request('batch', 'POST', { files });
    }

    public async presignPart(uploadId: string, partNumber: number, contentLength: number): Promise<PresignedPart> {
        return this.request('presign-part', 'POST', {
            part_number: assertPartNumber(partNumber),
            content_length: assertPositiveInteger(contentLength),
        }, assertUploadId(uploadId));
    }

    public async completeMultipart(uploadId: string, parts: CompletedPart[]): Promise<void> {
        await this.request('complete-multipart', 'POST', { parts }, assertUploadId(uploadId));
    }

    public async abortMultipart(uploadId: string): Promise<void> {
        await this.request('abort-multipart', 'DELETE', undefined, assertUploadId(uploadId));
    }

    public async deleteUpload(uploadId: string): Promise<void> {
        await this.request('delete', 'DELETE', undefined, assertUploadId(uploadId));
    }

    public async confirmSingle(uploadId: string): Promise<void> {
        await this.request('confirm-single', 'POST', undefined, assertUploadId(uploadId));
    }

    public async status(uploadId: string): Promise<UploadStatus> {
        return this.request('status', 'GET', undefined, assertUploadId(uploadId));
    }

    public async materializeAttachment(uploadId: string): Promise<NativeAttachment> {
        const attachment = await this.request<NativeAttachment>('prepare', 'POST', undefined, assertUploadId(uploadId));
        if (!attachment
            || !Number.isInteger(attachment.id)
            || attachment.id < 1
            || !/^[a-f0-9]{12}$/.test(attachment.hash)
            || typeof attachment.original_filename !== 'string'
            || typeof attachment.stored_filename !== 'string'
            || !['image/jpeg', 'video/mp4'].includes(attachment.mime_type)
            || !Number.isSafeInteger(attachment.size)
            || attachment.size < 1
        ) {
            throw new Error('G5 attachment preparation returned an invalid response');
        }

        return attachment;
    }

    private async request<T>(
        action: string,
        method: 'GET' | 'POST' | 'DELETE',
        body?: unknown,
        uploadId?: string,
    ): Promise<T> {
        const url = new URL(this.browserConfig.apiUrl, window.location.origin);
        url.searchParams.set('action', action);
        url.searchParams.set('bo_table', this.browserConfig.boardTable);
        if (uploadId) url.searchParams.set('upload_id', uploadId);
        const response = await this.fetcher(url.toString(), {
            method,
            credentials: 'same-origin',
            cache: 'no-store',
            redirect: 'error',
            headers: {
                accept: 'application/json',
                'content-type': 'application/json',
                'x-g7mb-csrf': this.browserConfig.csrfToken,
            },
            ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        });
        const decoded = await response.json().catch(() => null) as Envelope<T> | null;
        if (!response.ok || !decoded || decoded.success !== true || !('data' in decoded)) {
            throw new Error(safeMessage(decoded?.message));
        }

        return decoded.data as T;
    }
}

export function requiredConfiguration(): G5BrowserConfiguration {
    const value = window.G7MediaBoosterG5Config;
    if (!isSafeConfiguration(value)) {
        throw new Error('G5 MediaBooster browser configuration is unavailable');
    }
    return value;
}

function isSafeConfiguration(value: unknown): value is G5BrowserConfiguration {
    if (!value || typeof value !== 'object') return false;
    const candidate = value as Partial<G5BrowserConfiguration>;
    if (typeof candidate.apiUrl !== 'string'
        || typeof candidate.assetUrl !== 'string'
        || typeof candidate.boardTable !== 'string'
        || !/^[A-Za-z0-9_]{1,20}$/.test(candidate.boardTable)
        || typeof candidate.csrfToken !== 'string'
        || !/^[a-f0-9]{64}$/.test(candidate.csrfToken)
        || candidate.version !== '0.1.0'
    ) return false;
    try {
        const apiUrl = new URL(candidate.apiUrl, window.location.origin);
        const assetUrl = new URL(candidate.assetUrl, window.location.origin);
        return apiUrl.origin === window.location.origin && assetUrl.origin === window.location.origin;
    } catch {
        return false;
    }
}

function assertUploadId(value: string): string {
    if (!/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/.test(value)) {
        throw new TypeError('invalid upload id');
    }
    return value.toLowerCase();
}

function assertPartNumber(value: number): number {
    if (!Number.isInteger(value) || value < 1 || value > 10_000) throw new RangeError('invalid part number');
    return value;
}

function assertPositiveInteger(value: number): number {
    if (!Number.isSafeInteger(value) || value < 1) throw new RangeError('invalid content length');
    return value;
}

function safeMessage(value: unknown): string {
    return typeof value === 'string' && value.length > 0 && value.length <= 240
        ? value
        : '미디어 업로드 제어 요청에 실패했습니다.';
}

declare global {
    interface Window {
        G7MediaBoosterG5Config?: G5BrowserConfiguration;
    }
}
