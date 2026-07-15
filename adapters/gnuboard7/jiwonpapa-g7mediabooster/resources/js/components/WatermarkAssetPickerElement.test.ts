import { afterEach, describe, expect, it, vi } from 'vitest';
import { G7WatermarkAssetPickerElement } from './WatermarkAssetPickerElement';

const UPLOAD_ID = '018f47f0-4444-7444-8444-444444444444';

if (!customElements.get('g7-watermark-asset-picker-test')) {
    customElements.define('g7-watermark-asset-picker-test', G7WatermarkAssetPickerElement);
}

afterEach(() => {
    document.body.replaceChildren();
    delete (window as unknown as { G7Core?: unknown }).G7Core;
});

describe('watermark asset picker', () => {
    it('renders a validated same-origin asset and emits its upload id', async () => {
        const get = vi.fn().mockResolvedValue({
            success: true,
            data: {
                selected_upload_id: '',
                assets: [{
                    upload_id: UPLOAD_ID,
                    filename: 'logo.png',
                    source_bytes: 4096,
                    detected_content_type: 'image/png',
                    board_slug: 'notice',
                    created_at: '2026-07-16 12:00:00',
                }],
            },
        });
        (window as unknown as { G7Core?: unknown }).G7Core = { api: { get } };
        const picker = document.createElement('g7-watermark-asset-picker-test');
        const selected = vi.fn();
        picker.addEventListener('g7mb:watermark-selected', selected);
        document.body.append(picker);

        await vi.waitFor(() => expect(picker.shadowRoot?.querySelectorAll('input[type=radio]')).toHaveLength(1));
        expect(get).toHaveBeenCalledWith('/api/modules/jiwonpapa-g7mediabooster/admin/watermark-assets');
        expect(picker.shadowRoot?.textContent).toContain('logo.png');
        (picker.shadowRoot?.querySelector('input[type=radio]') as HTMLInputElement).click();

        expect(selected).toHaveBeenCalledTimes(1);
        expect((selected.mock.calls[0]?.[0] as CustomEvent).detail).toEqual({ uploadId: UPLOAD_ID });
    });

    it('fails closed when the server returns a non-supported source type', async () => {
        (window as unknown as { G7Core?: unknown }).G7Core = {
            api: {
                get: vi.fn().mockResolvedValue({
                    success: true,
                    data: {
                        selected_upload_id: '',
                        assets: [{
                            upload_id: UPLOAD_ID,
                            filename: 'logo.png',
                            source_bytes: 4096,
                            detected_content_type: 'image/avif',
                            board_slug: 'notice',
                            created_at: '2026-07-16 12:00:00',
                        }],
                    },
                }),
            },
        };
        const picker = document.createElement('g7-watermark-asset-picker-test');
        document.body.append(picker);

        await vi.waitFor(() => expect(picker.shadowRoot?.textContent).toContain('불러오지 못했습니다'));
        expect(picker.shadowRoot?.querySelector('input[type=radio]')).toBeNull();
    });
});
