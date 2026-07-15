import { afterEach, describe, expect, it } from 'vitest';
import { UploadTransportError, XhrUploadTransport } from './XhrUploadTransport';

class FakeXmlHttpRequest {
    public static latest: FakeXmlHttpRequest | null = null;
    public readonly upload: { onprogress: ((event: ProgressEvent) => void) | null } = { onprogress: null };
    public readonly headers = new Map<string, string>();
    public status = 200;
    public withCredentials = true;
    public onload: (() => void) | null = null;
    public onerror: (() => void) | null = null;
    public onabort: (() => void) | null = null;

    public constructor() {
        FakeXmlHttpRequest.latest = this;
    }

    public open(_method: string, _url: string, _async: boolean): void {}
    public setRequestHeader(name: string, value: string): void { this.headers.set(name.toLowerCase(), value); }
    public getResponseHeader(name: string): string | null { return name.toLowerCase() === 'etag' ? '"part-etag"' : null; }
    public send(body: Blob): void {
        this.upload.onprogress?.({ lengthComputable: true, loaded: body.size } as ProgressEvent);
        this.onload?.();
    }
    public abort(): void { this.onabort?.(); }
}

const originalXhr = globalThis.XMLHttpRequest;

afterEach(() => {
    globalThis.XMLHttpRequest = originalXhr;
    FakeXmlHttpRequest.latest = null;
});

describe('XhrUploadTransport', () => {
    it('lets the browser own Content-Length while setting signed safe headers', async () => {
        globalThis.XMLHttpRequest = FakeXmlHttpRequest as unknown as typeof XMLHttpRequest;
        const body = new Blob(['payload']);
        const result = await new XhrUploadTransport().put(
            'https://bucket.example.com/object?signature=secret',
            body,
            { 'content-length': String(body.size), 'content-type': 'image/jpeg', 'x-amz-meta-mode': 'raw' },
            new AbortController().signal,
            () => undefined,
        );

        expect(result.etag).toBe('"part-etag"');
        expect(FakeXmlHttpRequest.latest?.headers.has('content-length')).toBe(false);
        expect(FakeXmlHttpRequest.latest?.headers.get('content-type')).toBe('image/jpeg');
        expect(FakeXmlHttpRequest.latest?.withCredentials).toBe(false);
    });

    it('rejects a signed length that differs from the Blob size', async () => {
        await expect(new XhrUploadTransport().put(
            'https://bucket.example.com/object',
            new Blob(['payload']),
            { 'content-length': '999' },
            new AbortController().signal,
            () => undefined,
        )).rejects.toThrow('content-length');
    });

    it('rejects insecure non-loopback direct upload URLs', async () => {
        await expect(new XhrUploadTransport().put(
            'http://bucket.example.com/object',
            new Blob(['payload']),
            {},
            new AbortController().signal,
            () => undefined,
        )).rejects.toBeInstanceOf(UploadTransportError);
    });
});
