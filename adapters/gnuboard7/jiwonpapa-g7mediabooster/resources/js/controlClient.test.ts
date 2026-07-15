import { describe, expect, it, vi } from 'vitest';
import { G7MediaControlClient } from './controlClient';

describe('G7MediaControlClient', () => {
    it('uses only the same-origin G7 control route and unwraps its envelope', async () => {
        const api = {
            get: vi.fn().mockResolvedValue({ success: true, data: { upload_id: 'x', state: 'ready' } }),
            post: vi.fn().mockResolvedValue({ success: true, data: { batch_id: 'batch', uploads: [] } }),
            delete: vi.fn().mockResolvedValue({ success: true, data: null }),
        };
        const client = new G7MediaControlClient('free-board', api);

        const batch = await client.createBatch([]);

        expect(batch.batch_id).toBe('batch');
        expect(api.post).toHaveBeenCalledWith(
            '/api/modules/jiwonpapa-g7mediabooster/boards/free-board/uploads/batches',
            { files: [] },
        );

        await client.deleteUpload('018f47f0-3333-7333-8333-333333333333');
        expect(api.delete).toHaveBeenCalledWith(
            '/api/modules/jiwonpapa-g7mediabooster/boards/free-board/uploads/018f47f0-3333-7333-8333-333333333333',
        );
    });

    it('rejects a board slug that could alter the route', () => {
        expect(() => new G7MediaControlClient('../admin', {
            get: vi.fn(), post: vi.fn(), delete: vi.fn(),
        })).toThrow('board slug');
    });
});
