import { afterEach, describe, expect, it, vi } from 'vitest';
import { mountUploaderHandler, registerFormBridge } from './FormAttachmentBridge';

afterEach(() => {
    document.body.replaceChildren();
    delete (window as unknown as { G7Core?: unknown }).G7Core;
    vi.useRealTimers();
});

describe('G7 form attachment bridge', () => {
    it('mounts once and appends accepted native attachment ids to G7 form state', () => {
        document.body.innerHTML = '<div id="g7mb-user-uploader-mount"></div>';
        const setLocal = vi.fn();
        (window as unknown as { G7Core?: unknown }).G7Core = {
            state: {
                getLocal: () => ({ form: { attachment_ids: [3] } }),
                setLocal,
            },
        };

        const action = { params: { mountId: 'g7mb-user-uploader-mount', boardSlug: 'notice' } };
        mountUploaderHandler(action);
        mountUploaderHandler(action);

        const mount = document.getElementById('g7mb-user-uploader-mount') as HTMLElement;
        const uploader = mount.querySelector('g7-media-uploader') as HTMLElement;
        expect(mount.childElementCount).toBe(1);
        expect(uploader.getAttribute('board-slug')).toBe('notice');

        uploader.dispatchEvent(new CustomEvent('g7mb:state', { detail: { running: true } }));
        uploader.dispatchEvent(new CustomEvent('g7mb:complete', {
            detail: {
                batchId: 'batch',
                files: [
                    { state: 'accepted', attachment: { id: 7 } },
                    { state: 'accepted', attachment: { id: 7 } },
                    { state: 'accepted', attachment: { id: 8 } },
                    { state: 'failed', attachment: { id: 9 } },
                ],
            },
        }));

        expect(setLocal).toHaveBeenNthCalledWith(2, { g7mbUploading: true }, { merge: 'deep', render: true });
        expect(setLocal).toHaveBeenLastCalledWith({
            'form.attachment_ids': [3, 7, 8],
            g7mbUploading: false,
            hasChanges: true,
        }, { merge: 'deep', render: true });
    });

    it('fails closed for invalid layout wiring or an unavailable G7 state bridge', () => {
        document.body.innerHTML = '<div id="safe-mount"></div>';
        expect(() => mountUploaderHandler({
            params: { mountId: '../unsafe', boardSlug: 'notice' },
        })).toThrow('mount id');
        expect(() => mountUploaderHandler({
            params: { mountId: 'safe-mount', boardSlug: '../admin' },
        })).toThrow('board slug');
        expect(() => mountUploaderHandler({
            params: { mountId: 'safe-mount', boardSlug: 'notice' },
        })).toThrow('state bridge');
    });

    it('registers the namespaced module handler', () => {
        const registerHandler = vi.fn();
        (window as unknown as { G7Core?: unknown }).G7Core = {
            getActionDispatcher: () => ({ registerHandler }),
        };

        registerFormBridge();

        expect(registerHandler).toHaveBeenCalledWith(
            'jiwonpapa-g7mediabooster.mountUploader',
            mountUploaderHandler,
            { category: 'module', source: 'jiwonpapa-g7mediabooster' },
        );
    });

    it('ignores malformed completion events', () => {
        document.body.innerHTML = '<div id="safe-mount"></div>';
        const setLocal = vi.fn();
        (window as unknown as { G7Core?: unknown }).G7Core = {
            state: { getLocal: () => ({ form: {} }), setLocal },
        };
        mountUploaderHandler({ params: { mountId: 'safe-mount', boardSlug: 'notice' } });
        const uploader = document.querySelector('g7-media-uploader') as HTMLElement;

        uploader.dispatchEvent(new CustomEvent('g7mb:complete', {
            detail: { batchId: 'batch', files: [null, { state: 'accepted', attachment: { id: -1 } }] },
        }));

        expect(setLocal).toHaveBeenCalledTimes(1);
    });
});
