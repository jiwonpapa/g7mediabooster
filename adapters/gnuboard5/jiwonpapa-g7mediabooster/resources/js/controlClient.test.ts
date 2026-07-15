import { describe, expect, it, vi } from 'vitest';
import { G5MediaControlClient, type G5BrowserConfiguration } from './controlClient';

const browserConfiguration: G5BrowserConfiguration = {
    apiUrl: '/plugin/g7mediabooster/api.php',
    assetUrl: '/plugin/g7mediabooster/assets/uploader.iife.js',
    boardTable: 'free',
    csrfToken: 'a'.repeat(64),
    version: '0.1.0',
};

describe('G5MediaControlClient', () => {
    it('uses same-origin credentials, CSRF, and scoped action query', async () => {
        const fetcher = vi.fn(async () => new Response(JSON.stringify({
            success: true,
            data: {
                enabled: true,
                max_files: 10,
                max_file_size_bytes: 1024,
                max_parallel_files: 4,
                max_parallel_parts: 2,
                max_part_retries: 3,
                status_poll_interval_ms: 1500,
            },
        }), { status: 200, headers: { 'content-type': 'application/json' } }));

        const configuration = await new G5MediaControlClient(browserConfiguration, fetcher).configuration();

        expect(configuration.enabled).toBe(true);
        expect(fetcher).toHaveBeenCalledOnce();
        const [url, init] = fetcher.mock.calls[0] as unknown as [string, RequestInit];
        expect(url).toContain('action=configuration');
        expect(url).toContain('bo_table=free');
        expect(init.credentials).toBe('same-origin');
        expect((init.headers as Record<string, string>)['x-g7mb-csrf']).toBe('a'.repeat(64));
    });

    it('never sends the original filename to a different origin itself', async () => {
        const fetcher = vi.fn(async (_url: RequestInfo | URL, init?: RequestInit) => {
            const body = JSON.parse(String(init?.body)) as { files: Array<Record<string, unknown>> };
            expect(body.files[0]?.original_filename).toBe('local-only.jpg');
            return new Response(JSON.stringify({
                success: true,
                data: { batch_id: '018f47f0-1111-7111-8111-111111111111', uploads: [] },
            }), { status: 201, headers: { 'content-type': 'application/json' } });
        });
        const client = new G5MediaControlClient(browserConfiguration, fetcher);

        await client.createBatch([{
            client_ref: 'file_1',
            original_filename: 'local-only.jpg',
            declared_kind: 'image',
            content_length: 10,
            content_type_hint: 'image/jpeg',
        }]);

        expect(fetcher).toHaveBeenCalledOnce();
        expect(new URL(fetcher.mock.calls[0]?.[0] as string).origin).toBe(window.location.origin);
    });
});
