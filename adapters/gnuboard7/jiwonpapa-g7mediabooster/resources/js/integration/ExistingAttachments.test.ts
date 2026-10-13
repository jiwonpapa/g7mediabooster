import { describe, expect, it, vi } from 'vitest';
import { ExistingAttachments } from './ExistingAttachments';

const base = '/api/modules/sirsoft-board/boards/gallery/attachments';
const files = [
    { id: 3, original_filename: '<img src=x onerror=alert(1)>', preview_url: null },
    { id: 7, original_filename: 'second.jpg', preview_url: '/api/modules/sirsoft-board/boards/gallery/attachment/abcdefgh1234/preview' },
];

function manager(disabled = false) {
    const api = { delete: vi.fn().mockResolvedValue({ success: true }), patch: vi.fn().mockResolvedValue({ success: true }) };
    const changed = vi.fn();
    const busy = vi.fn();
    const list = new ExistingAttachments('gallery', files,
        { deleteEndpoint: base+'/:id', reorderEndpoint: base+'/reorder' },
        disabled, changed, busy, api);
    return { list, api, changed, busy };
}

describe('existing G7 attachment management', () => {
    it('shows both existing files, escapes names and scopes preview URLs', () => {
        const { list } = manager();
        expect(list.ids()).toEqual([3, 7]);
        expect(list.element.querySelectorAll('li')).toHaveLength(2);
        expect(list.element.textContent).toContain('<img src=x onerror=alert(1)>');
        expect(list.element.querySelectorAll('img')).toHaveLength(1);
        expect(() => new ExistingAttachments('gallery', files,
            { deleteEndpoint: 'https://evil.test/delete/:id', reorderEndpoint: base+'/reorder' },
            false, vi.fn(), vi.fn())).toThrow('endpoint');
        expect(() => new ExistingAttachments('gallery', [{ id: 0 }],
            { deleteEndpoint: base+'/:id', reorderEndpoint: base+'/reorder' },
            false, vi.fn(), vi.fn())).toThrow('attachment');
    });

    it('deletes through native permissions and updates IDs only after success', async () => {
        const { list, api, changed, busy } = manager();
        vi.spyOn(window, 'confirm').mockReturnValue(false);
        await list.remove(3);
        expect(api.delete).not.toHaveBeenCalled();
        vi.spyOn(window, 'confirm').mockReturnValue(true);
        await list.remove(3);
        expect(api.delete).toHaveBeenCalledWith(base+'/3');
        expect(list.ids()).toEqual([7]);
        expect(changed).toHaveBeenCalledWith([7]);
        expect(busy).toHaveBeenLastCalledWith(false);
    });

    it('retains existing IDs and renders an error when deletion or reorder fails', async () => {
        const { list, api, changed } = manager();
        vi.spyOn(window, 'confirm').mockReturnValue(true);
        api.delete.mockRejectedValue(new Error('denied'));
        await list.remove(3);
        api.patch.mockResolvedValue({ success: false });
        await list.move(7, -1);
        expect(list.ids()).toEqual([3, 7]);
        expect(changed).not.toHaveBeenCalled();
        expect(list.element.textContent).toContain('변경하지 못했습니다');
    });

    it('reorders using the native payload and prevents concurrent mutations', async () => {
        const { list, api, changed } = manager();
        let finish: (value: { success: boolean }) => void = () => undefined;
        api.patch.mockImplementation(() => new Promise(resolve => { finish = resolve; }));
        const moving = list.move(7, -1);
        await list.move(3, 1);
        expect(api.patch).toHaveBeenCalledTimes(1);
        expect(list.ids()).toEqual([3, 7]);
        finish({ success: true });
        await moving;
        expect(api.patch).toHaveBeenCalledWith(base+'/reorder', { order: [{ id: 7, order: 1 }, { id: 3, order: 2 }] });
        expect(changed).toHaveBeenCalledWith([7, 3]);
    });

    it('keeps read-only attachments visible without allowing mutations and deduplicates new files', async () => {
        const { list, api } = manager(true);
        await list.remove(3);
        await list.move(7, -1);
        expect(api.delete).not.toHaveBeenCalled();
        expect(api.patch).not.toHaveBeenCalled();
        list.add([{ id: 7, original_filename: 'second.jpg' }, { id: 8, original_filename: 'video.mp4' }]);
        expect(list.ids()).toEqual([3, 7, 8]);
        expect(Array.from(list.element.querySelectorAll('button')).every(button => button.disabled)).toBe(true);
    });
});
