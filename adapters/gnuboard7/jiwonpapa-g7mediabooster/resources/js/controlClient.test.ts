import { describe, expect, it, vi } from 'vitest';
import { G7MediaControlClient } from './controlClient';
import type { NativeAttachment } from './types';

const uploadId = '018f47f0-4444-7444-8444-444444444444';
const hash = 'aBcDeF123456';
const url = `/api/modules/sirsoft-board/boards/gallery/attachment/${hash}`;
const attachment: NativeAttachment = {
    id: 37, hash, original_filename: 'image.png', stored_filename: 'master.jpg',
    mime_type: 'image/jpeg', size: 128, url, preview_url: `${url}/preview`, order: 1,
    created_at: '2026-10-12T08:00:00Z',
};

function client(value: NativeAttachment): G7MediaControlClient {
    const response = { success: true, data: { data: value } };
    return new G7MediaControlClient('gallery', {
        get: async <T>() => ({} as T),
        post: async <T>() => (response as T),
        delete: async <T>() => ({} as T),
    });
}

describe('native G7 attachment response', () => {
    it('accepts the exact native board URL emitted by the PHP bridge', async () => {
        await expect(client(attachment).materializeAttachment(uploadId)).resolves.toEqual(attachment);
    });

    it.each([
        { url: 'https://external.example/image.jpg' },
        { url: url.replace('/gallery/', '/other/') },
        { url: url.replace(hash, 'Other1234567') },
        { preview_url: `${url}/preview?redirect=external` },
        { preview_url: '/api/modules/jiwonpapa-g7mediabooster/arbitrary' },
    ])('rejects delivery outside the exact board/hash contract: %o', async (override) => {
        await expect(client({ ...attachment, ...override }).materializeAttachment(uploadId))
            .rejects.toThrow('invalid response');
    });
});

describe('G7MediaControlClient control boundary', () => {
    it('uses the same-origin route and unwraps batch/delete envelopes', async () => {
        const api = {
            get: vi.fn(), post: vi.fn().mockResolvedValue({success:true,data:{batch_id:'batch',uploads:[]}}),
            delete: vi.fn().mockResolvedValue({success:true,data:null}),
        };
        const control = new G7MediaControlClient('free-board', api);
        expect((await control.createBatch([])).batch_id).toBe('batch');
        expect(api.post).toHaveBeenCalledWith(
            '/api/modules/jiwonpapa-g7mediabooster/boards/free-board/uploads/batches', {files:[]});
        await control.deleteUpload(uploadId);
        expect(api.delete).toHaveBeenCalledWith(
            `/api/modules/jiwonpapa-g7mediabooster/boards/free-board/uploads/${uploadId}`);
    });

    it('rejects route-altering board slugs', () => {
        expect(() => new G7MediaControlClient('../admin', {
            get:vi.fn(),post:vi.fn(),delete:vi.fn(),
        })).toThrow('board slug');
    });

    it('materializes through the control route while accepting native delivery paths', async () => {
        const api = {get:vi.fn(),delete:vi.fn(),post:vi.fn().mockResolvedValue({
            success:true,data:{data:attachment},
        })};
        expect((await new G7MediaControlClient('gallery',api).materializeAttachment(uploadId)).id).toBe(37);
        expect(api.post).toHaveBeenCalledWith(
            `/api/modules/jiwonpapa-g7mediabooster/boards/gallery/uploads/${uploadId}/attachment`);
    });
});
