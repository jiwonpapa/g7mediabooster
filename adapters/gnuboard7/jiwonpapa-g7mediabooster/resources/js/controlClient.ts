import type {
    CompletedPart,
    MediaControlClient,
    PresignedPart,
    PublicUploaderConfiguration,
    UploadBatch,
    UploadFileIntent,
    UploadStatus,
} from './types';

interface G7Api {
    get<T>(url: string): Promise<T>;
    post<T>(url: string, data?: unknown): Promise<T>;
    delete<T>(url: string): Promise<T>;
}

interface G7Envelope<T> {
    success: boolean;
    message?: string;
    data: T;
}

export class G7MediaControlClient implements MediaControlClient {
    private readonly basePath: string;

    public constructor(boardSlug: string, private readonly api: G7Api = requiredG7Api()) {
        if (!/^[A-Za-z0-9_-]+$/.test(boardSlug)) {
            throw new TypeError('invalid board slug');
        }
        this.basePath = `/api/modules/jiwonpapa-g7mediabooster/boards/${encodeURIComponent(boardSlug)}/uploads`;
    }

    public async configuration(): Promise<PublicUploaderConfiguration> {
        return unwrap(await this.api.get<G7Envelope<PublicUploaderConfiguration>>(`${this.basePath}/configuration`));
    }

    public async createBatch(files: UploadFileIntent[]): Promise<UploadBatch> {
        return unwrap(await this.api.post<G7Envelope<UploadBatch>>(`${this.basePath}/batches`, { files }));
    }

    public async presignPart(uploadId: string, partNumber: number, contentLength: number): Promise<PresignedPart> {
        return unwrap(await this.api.post<G7Envelope<PresignedPart>>(
            `${this.basePath}/${assertUploadId(uploadId)}/parts/${assertPartNumber(partNumber)}/presign`,
            { content_length: contentLength },
        ));
    }

    public async completeMultipart(uploadId: string, parts: CompletedPart[]): Promise<void> {
        await this.api.post(`${this.basePath}/${assertUploadId(uploadId)}/multipart/complete`, { parts });
    }

    public async abortMultipart(uploadId: string): Promise<void> {
        await this.api.delete(`${this.basePath}/${assertUploadId(uploadId)}/multipart`);
    }

    public async deleteUpload(uploadId: string): Promise<void> {
        await this.api.delete(`${this.basePath}/${assertUploadId(uploadId)}`);
    }

    public async confirmSingle(uploadId: string): Promise<void> {
        await this.api.post(`${this.basePath}/${assertUploadId(uploadId)}/complete`);
    }

    public async status(uploadId: string): Promise<UploadStatus> {
        return unwrap(await this.api.get<G7Envelope<UploadStatus>>(`${this.basePath}/${assertUploadId(uploadId)}`));
    }
}

function unwrap<T>(response: G7Envelope<T>): T {
    if (!response || response.success !== true || response.data === undefined) {
        throw new Error(response?.message || 'G7 control API returned an invalid response');
    }
    return response.data;
}

function requiredG7Api(): G7Api {
    const api = (window as Window & { G7Core?: { api?: G7Api } }).G7Core?.api;
    if (!api) {
        throw new Error('G7Core.api is not available');
    }
    return api;
}

function assertUploadId(uploadId: string): string {
    if (!/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/.test(uploadId)) {
        throw new TypeError('invalid upload id');
    }
    return uploadId;
}

function assertPartNumber(partNumber: number): number {
    if (!Number.isInteger(partNumber) || partNumber < 1 || partNumber > 10_000) {
        throw new RangeError('invalid part number');
    }
    return partNumber;
}

declare global {
    interface Window {
        G7Core?: { api?: G7Api };
    }
}
