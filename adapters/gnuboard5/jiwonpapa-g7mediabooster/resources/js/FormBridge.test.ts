import { beforeEach, describe, expect, it, vi } from 'vitest';
import { mountG5Uploader } from './FormBridge';
import { G5UploaderElement } from './G5UploaderElement';

describe('Gnuboard5 form bridge', () => {
    beforeEach(() => {
        document.body.innerHTML = `<form action="/bbs/write_update.php">
            <div class="bo_w_flie"><div class="file_wr"><label for="bf_file_1">파일</label><input id="bf_file_1" type="file" name="bf_file[]"></div></div>
            <button type="submit">저장</button>
        </form>`;
        window.G7MediaBoosterG5Config = {
            apiUrl: '/plugin/g7mediabooster/api.php',
            assetUrl: '/plugin/g7mediabooster/assets/uploader.iife.js',
            boardTable: 'free',
            csrfToken: 'a'.repeat(64),
            version: '0.1.0',
        };
        if (!customElements.get('g7mb-g5-uploader')) {
            customElements.define('g7mb-g5-uploader', G5UploaderElement);
        }
    });

    it('keeps the native uploader untouched when the adapter is disabled', async () => {
        vi.stubGlobal('fetch', vi.fn(async () => response(false)));

        expect(await mountG5Uploader()).toBe(false);
        const native = document.querySelector<HTMLInputElement>('input[name="bf_file[]"]');
        expect(native?.disabled).toBe(false);
        expect(document.querySelector('g7mb-g5-uploader')).toBeNull();
    });

    it('replaces only native file inputs after enabled configuration succeeds', async () => {
        vi.stubGlobal('fetch', vi.fn(async () => response(true)));

        expect(await mountG5Uploader()).toBe(true);
        const native = document.querySelector<HTMLInputElement>('input[name="bf_file[]"]');
        expect(native?.disabled).toBe(true);
        expect(native?.closest<HTMLElement>('.file_wr')?.hidden).toBe(true);
        expect(document.querySelector('g7mb-g5-uploader')).not.toBeNull();
        expect(document.querySelector<HTMLInputElement>('input[name="g7mb_upload_ids"]')?.value).toBe('');
    });

    it('clears superseded ready uploads when a new visible selection replaces them', async () => {
        const fetcher = vi.fn(async (_input: RequestInfo | URL) => response(true));
        vi.stubGlobal('fetch', fetcher);
        await mountG5Uploader();
        const hidden = document.querySelector<HTMLInputElement>('input[name="g7mb_upload_ids"]');
        const uploader = document.querySelector('g7mb-g5-uploader');
        expect(hidden).not.toBeNull();
        expect(uploader).not.toBeNull();
        hidden!.value = '018f47f0-2222-7222-8222-222222222222';

        uploader!.dispatchEvent(new CustomEvent('g7mb:selection'));

        expect(hidden!.value).toBe('');
        await vi.waitFor(() => expect(fetcher).toHaveBeenCalledTimes(3));
        expect(fetcher.mock.calls.some((call) => String(call[0]).includes('action=delete'))).toBe(true);
    });
});

function response(enabled: boolean): Response {
    return new Response(JSON.stringify({
        success: true,
        data: {
            enabled,
            max_files: 10,
            max_file_size_bytes: 1024,
            max_parallel_files: 4,
            max_parallel_parts: 2,
            max_part_retries: 3,
            status_poll_interval_ms: 1500,
        },
    }), { status: 200, headers: { 'content-type': 'application/json' } });
}
