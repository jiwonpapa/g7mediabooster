import { describe, expect, it, vi } from 'vitest';
import type {
    CompletedPart,
    DirectUploadTransport,
    MediaControlClient,
    NativeAttachment,
    PresignedPart,
    PublicUploaderConfiguration,
    UploadBatch,
    UploadFileIntent,
    UploadStatus,
} from '../types';
import { MultiUploader } from './MultiUploader';

class FakeControl implements MediaControlClient {
    public mode: 'single_put' | 'multipart' = 'single_put';
    public partSize = 5 * 1024 * 1024;
    public readonly confirmed: string[] = [];
    public readonly aborted: string[] = [];
    public readonly presignedLengths: number[] = [];
    public batchRequests = 0;
    public completed: CompletedPart[] = [];
    public requestedFiles: UploadFileIntent[] = [];
    public materialized = 0;
    public expectedTransfersBeforeStatus = 0;

    public async configuration(): Promise<PublicUploaderConfiguration> {
        return {
            enabled: true,
            max_files: 100,
            max_file_size_bytes: 5 * 1024 * 1024 * 1024,
            max_parallel_files: 8,
            max_parallel_parts: 4,
            max_part_retries: 0,
            status_poll_interval_ms: 1500,
        };
    }

    public async createBatch(files: UploadFileIntent[]): Promise<UploadBatch> {
        this.batchRequests += 1;
        this.requestedFiles = files;
        return {
            batch_id: '018f47f0-1111-7111-8111-111111111111',
            uploads: files.map((file, index) => {
                const requiredHeaders: Record<string, string> = this.mode === 'single_put'
                    ? { 'content-length': String(file.content_length) }
                    : {};
                return {
                    client_ref: file.client_ref,
                    upload_id: `018f47f0-2222-7222-8222-${String(index + 1).padStart(12, '0')}`,
                    method: this.mode,
                    part_size_bytes: this.mode === 'multipart' ? this.partSize : null,
                    upload_url: this.mode === 'single_put' ? `https://bucket.example.com/single-${index}` : null,
                    required_headers: requiredHeaders,
                    expires_at: '2030-01-01T00:00:00Z',
                };
            }),
        };
    }

    public async presignPart(_uploadId: string, partNumber: number, contentLength: number): Promise<PresignedPart> {
        this.presignedLengths.push(contentLength);
        return {
            part_number: partNumber,
            upload_url: `https://bucket.example.com/part-${partNumber}`,
            required_headers: { 'content-length': String(contentLength) },
            expires_at: '2030-01-01T00:00:00Z',
        };
    }

    public async completeMultipart(_uploadId: string, parts: CompletedPart[]): Promise<void> {
        this.completed = parts;
    }

    public async abortMultipart(uploadId: string): Promise<void> {
        this.aborted.push(uploadId);
    }

    public async deleteUpload(_uploadId: string): Promise<void> {}

    public async confirmSingle(uploadId: string): Promise<void> {
        this.confirmed.push(uploadId);
    }

    public async status(uploadId: string): Promise<UploadStatus> {
        if (this.expectedTransfersBeforeStatus > 0 && this.confirmed.length !== this.expectedTransfersBeforeStatus) {
            throw new Error('status polling started before the bounded transfer phase completed');
        }
        return {
            upload_id: uploadId,
            state: 'ready',
            detected_content_type: 'image/jpeg',
            error_code: null,
            deletion_pending: false,
            derivatives: [
                { preset_id: 'v1', variant: 'master', url_path: '/master.jpg', delivery_url: '', content_type: 'image/jpeg', byte_len: 1024 },
                { preset_id: 'v1', variant: 'thumbnail', url_path: '/thumbnail.jpg', delivery_url: '', content_type: 'image/jpeg', byte_len: 512 },
            ],
        };
    }

    public async materializeAttachment(uploadId: string): Promise<NativeAttachment> {
        this.materialized += 1;
        return {
            id: this.materialized,
            hash: `hash${String(this.materialized).padStart(8, '0')}`,
            original_filename: 'image.jpg',
            stored_filename: `${uploadId}.jpg`,
            mime_type: 'image/jpeg',
            size: 1024,
            url: `/api/modules/jiwonpapa-g7mediabooster/attachment/${this.materialized}/master`,
            preview_url: `/api/modules/jiwonpapa-g7mediabooster/attachment/${this.materialized}/thumbnail`,
            order: 0,
            created_at: null,
        };
    }
}

class TrackingTransport implements DirectUploadTransport {
    public active = 0;
    public maxActive = 0;
    public etag: string | null = '"etag"';

    public async put(
        _url: string,
        body: Blob,
        _requiredHeaders: Record<string, string>,
        signal: AbortSignal,
        onProgress: (loaded: number) => void,
    ): Promise<{ etag: string | null }> {
        signal.throwIfAborted();
        this.active += 1;
        this.maxActive = Math.max(this.maxActive, this.active);
        try {
            await new Promise<void>((resolve, reject) => {
                const timer = window.setTimeout(resolve, 5);
                signal.addEventListener('abort', () => {
                    window.clearTimeout(timer);
                    reject(signal.reason ?? new DOMException('aborted', 'AbortError'));
                }, { once: true });
            });
            onProgress(body.size);
            return { etag: this.etag };
        } finally {
            this.active -= 1;
        }
    }
}

describe('MultiUploader', () => {
    it('schedules a 100-file batch through one control request and bounded connections', async () => {
        vi.useFakeTimers();
        const control = new FakeControl();
        control.expectedTransfersBeforeStatus = 100;
        const transport = new TrackingTransport();
        const files = Array.from(
            { length: 100 },
            (_, index) => fileOfSize(1024, `image-${index}.jpg`, 'image/jpeg'),
        );
        const startedAt = Date.now();

        try {
            const pending = new MultiUploader(control, transport).upload(files, {
                maxParallelFiles: 16,
                maxConnections: 8,
                maxRetries: 0,
            });
            await vi.runAllTimersAsync();
            const result = await pending;

            expect(control.batchRequests).toBe(1);
            expect(control.requestedFiles[0]?.original_filename).toBe('image-0.jpg');
            expect(transport.maxActive).toBe(8);
            expect(control.confirmed).toHaveLength(100);
            expect(control.materialized).toBe(100);
            expect(Date.now() - startedAt).toBeGreaterThanOrEqual(24_875);
            expect(result.files).toHaveLength(100);
            expect(result.files.every((file) => file.state === 'accepted')).toBe(true);
            expect(result.files.every((file) => file.attachment !== null)).toBe(true);
        } finally {
            vi.useRealTimers();
        }
    });

    it('rejects 101 files before creating a control-plane batch', async () => {
        const control = new FakeControl();
        const transport = new TrackingTransport();
        const files = Array.from(
            { length: 101 },
            (_, index) => fileOfSize(1, `image-${index}.jpg`, 'image/jpeg'),
        );

        await expect(new MultiUploader(control, transport).upload(files)).rejects.toThrow('1-100');
        expect(control.batchRequests).toBe(0);
        expect(transport.maxActive).toBe(0);
    });

    it('rejects media outside the published release formats before reservation', async () => {
        const control = new FakeControl();
        const uploader = new MultiUploader(control, new TrackingTransport());

        await expect(uploader.upload([
            fileOfSize(1024, 'clip.webm', 'video/webm'),
        ])).rejects.toThrow('unsupported media type');
        expect(control.batchRequests).toBe(0);
    });

    it('accepts QuickTime MOV as a release video container', async () => {
        const control = new FakeControl();
        const uploader = new MultiUploader(control, new TrackingTransport());

        const result = await uploader.upload([
            fileOfSize(1024, 'clip.mov', 'video/quicktime'),
        ]);

        expect(control.requestedFiles[0]?.declared_kind).toBe('video');
        expect(control.requestedFiles[0]?.content_type_hint).toBe('video/quicktime');
        expect(result.files[0]?.state).toBe('accepted');
    });

    it('caps direct PUT connections across multiple files', async () => {
        const control = new FakeControl();
        const transport = new TrackingTransport();
        const files = Array.from({ length: 6 }, (_, index) => fileOfSize(1024, `image-${index}.jpg`, 'image/jpeg'));

        const result = await new MultiUploader(control, transport).upload(files, {
            maxParallelFiles: 6,
            maxConnections: 2,
            maxRetries: 0,
        });

        expect(transport.maxActive).toBe(2);
        expect(control.confirmed).toHaveLength(6);
        expect(result.files.every((file) => file.state === 'accepted')).toBe(true);
    });

    it('uploads one large file as bounded ordered multipart parts', async () => {
        const control = new FakeControl();
        control.mode = 'multipart';
        const transport = new TrackingTransport();
        const file = fileOfSize(11 * 1024 * 1024, 'video.mp4', 'video/mp4');

        const result = await new MultiUploader(control, transport).upload([file], {
            maxParallelParts: 2,
            maxConnections: 2,
            maxRetries: 0,
        });

        expect(transport.maxActive).toBe(2);
        expect(control.presignedLengths).toEqual([5 * 1024 * 1024, 5 * 1024 * 1024, 1024 * 1024]);
        expect(control.completed.map((part) => part.part_number)).toEqual([1, 2, 3]);
        expect(result.files[0]?.state).toBe('accepted');
    });

    it('aborts multipart when CORS does not expose ETag', async () => {
        const control = new FakeControl();
        control.mode = 'multipart';
        const transport = new TrackingTransport();
        transport.etag = null;

        const result = await new MultiUploader(control, transport).upload([
            fileOfSize(6 * 1024 * 1024, 'video.mp4', 'video/mp4'),
        ], { maxRetries: 0 });

        expect(control.aborted).toHaveLength(1);
        expect(control.completed).toEqual([]);
        expect(result.files[0]?.state).toBe('failed');
        expect(result.files[0]?.error).toContain('ETag');
    });

    it('reports cancellation without confirming the object', async () => {
        const control = new FakeControl();
        const transport = new TrackingTransport();
        const controller = new AbortController();
        const progress = vi.fn();
        const promise = new MultiUploader(control, transport).upload([
            fileOfSize(1024, 'image.jpg', 'image/jpeg'),
        ], { signal: controller.signal, onProgress: progress });

        controller.abort(new DOMException('cancelled', 'AbortError'));
        const result = await promise;

        expect(result.files[0]?.state).toBe('cancelled');
        expect(control.confirmed).toEqual([]);
        expect(progress).toHaveBeenCalled();
    });
});

function fileOfSize(size: number, name: string, type: string): File {
    return new File([new Uint8Array(size)], name, { type });
}
