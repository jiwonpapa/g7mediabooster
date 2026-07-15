import type { DirectUploadTransport } from '../types';

const NEVER_SET_HEADERS = new Set(['authorization', 'cookie', 'content-length', 'host', 'origin', 'referer']);

export class UploadTransportError extends Error {
    public constructor(message: string, public readonly status: number | null) {
        super(message);
        this.name = 'UploadTransportError';
    }

    public get transient(): boolean {
        return this.status === null || this.status === 408 || this.status === 429 || (this.status >= 500 && this.status <= 599);
    }
}

export class XhrUploadTransport implements DirectUploadTransport {
    public async put(
        url: string,
        body: Blob,
        requiredHeaders: Record<string, string>,
        signal: AbortSignal,
        onProgress: (loaded: number) => void,
    ): Promise<{ etag: string | null }> {
        assertDirectUploadUrl(url);
        assertRequiredContentLength(requiredHeaders, body.size);
        signal.throwIfAborted();

        return new Promise((resolve, reject) => {
            const xhr = new XMLHttpRequest();
            const abort = (): void => xhr.abort();
            const cleanup = (): void => signal.removeEventListener('abort', abort);
            xhr.open('PUT', url, true);
            xhr.withCredentials = false;
            for (const [name, value] of Object.entries(requiredHeaders)) {
                const normalized = name.toLowerCase();
                if (NEVER_SET_HEADERS.has(normalized)) {
                    if (normalized === 'content-length') {
                        continue;
                    }
                    cleanup();
                    reject(new UploadTransportError(`unsafe required header: ${normalized}`, null));
                    return;
                }
                xhr.setRequestHeader(name, value);
            }
            xhr.upload.onprogress = (event): void => {
                if (event.lengthComputable) {
                    onProgress(Math.min(body.size, event.loaded));
                }
            };
            xhr.onload = (): void => {
                cleanup();
                if (xhr.status >= 200 && xhr.status < 300) {
                    onProgress(body.size);
                    resolve({ etag: xhr.getResponseHeader('ETag') });
                    return;
                }
                reject(new UploadTransportError(`direct upload failed with HTTP ${xhr.status}`, xhr.status));
            };
            xhr.onerror = (): void => {
                cleanup();
                reject(new UploadTransportError('direct upload failed; verify object-store CORS', null));
            };
            xhr.onabort = (): void => {
                cleanup();
                reject(signal.reason ?? new DOMException('Upload aborted', 'AbortError'));
            };
            signal.addEventListener('abort', abort, { once: true });
            xhr.send(body);
        });
    }
}

function assertDirectUploadUrl(value: string): void {
    let url: URL;
    try {
        url = new URL(value);
    } catch {
        throw new UploadTransportError('invalid direct-upload URL', null);
    }
    const host = url.hostname.replace(/^\[|\]$/g, '').toLowerCase();
    const loopback = host === 'localhost' || host === '::1' || host.startsWith('127.');
    if (url.username || url.password || (url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback))) {
        throw new UploadTransportError('direct-upload URL must use HTTPS or literal loopback HTTP', null);
    }
}

function assertRequiredContentLength(headers: Record<string, string>, actual: number): void {
    const pair = Object.entries(headers).find(([name]) => name.toLowerCase() === 'content-length');
    if (!pair) {
        return;
    }
    const expected = Number(pair[1]);
    if (!Number.isSafeInteger(expected) || expected !== actual) {
        throw new UploadTransportError('signed content-length does not match upload body', null);
    }
}
