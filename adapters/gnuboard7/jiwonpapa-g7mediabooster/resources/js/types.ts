export type DeclaredKind = 'image' | 'video';
export type UploadMethod = 'single_put' | 'multipart';
export type UploadLifecycleState =
    | 'queued'
    | 'uploading'
    | 'verifying'
    | 'accepted'
    | 'failed'
    | 'cancelled';

export interface UploadFileIntent {
    client_ref: string;
    declared_kind: DeclaredKind;
    content_length: number;
    content_type_hint: string;
}

export interface UploadIntent {
    client_ref: string;
    upload_id: string;
    method: UploadMethod;
    part_size_bytes: number | null;
    upload_url: string | null;
    required_headers: Record<string, string>;
    expires_at: string;
}

export interface UploadBatch {
    batch_id: string;
    uploads: UploadIntent[];
}

export interface PresignedPart {
    part_number: number;
    upload_url: string;
    required_headers: Record<string, string>;
    expires_at: string;
}

export interface CompletedPart {
    part_number: number;
    etag: string;
}

export interface UploadStatus {
    upload_id: string;
    state: string;
    detected_content_type: string | null;
    error_code: string | null;
    deletion_pending: boolean;
    derivatives: Array<{
        preset_id: string;
        variant: string;
        url_path: string;
        content_type: string;
        byte_len: number;
    }>;
}

export interface PublicUploaderConfiguration {
    enabled: boolean;
    max_files: number;
    max_file_size_bytes: number;
    max_parallel_files: number;
    max_parallel_parts: number;
    max_part_retries: number;
    status_poll_interval_ms: number;
}

export interface MediaControlClient {
    configuration(): Promise<PublicUploaderConfiguration>;
    createBatch(files: UploadFileIntent[]): Promise<UploadBatch>;
    presignPart(uploadId: string, partNumber: number, contentLength: number): Promise<PresignedPart>;
    completeMultipart(uploadId: string, parts: CompletedPart[]): Promise<void>;
    abortMultipart(uploadId: string): Promise<void>;
    deleteUpload(uploadId: string): Promise<void>;
    confirmSingle(uploadId: string): Promise<void>;
    status(uploadId: string): Promise<UploadStatus>;
}

export interface DirectUploadTransport {
    put(
        url: string,
        body: Blob,
        requiredHeaders: Record<string, string>,
        signal: AbortSignal,
        onProgress: (loaded: number) => void,
    ): Promise<{ etag: string | null }>;
}

export interface FileUploadProgress {
    clientRef: string;
    file: File;
    uploadId: string | null;
    state: UploadLifecycleState;
    bytesSent: number;
    totalBytes: number;
    percent: number;
    error?: string;
}

export interface FileUploadResult {
    clientRef: string;
    uploadId: string | null;
    file: File;
    state: 'accepted' | 'failed' | 'cancelled';
    error?: string;
}

export interface UploadBatchResult {
    batchId: string;
    files: FileUploadResult[];
}
