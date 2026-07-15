import type { UploadBatchResult } from '../../../../gnuboard7/jiwonpapa-g7mediabooster/resources/js/types';
import { G5MediaControlClient } from './controlClient';
import { G5UploaderElement } from './G5UploaderElement';

export async function mountG5Uploader(documentRoot: Document = document): Promise<boolean> {
    const form = findWriteForm(documentRoot);
    if (!form || form.dataset.g7mbMounted === 'true') return false;

    const controlClient = new G5MediaControlClient();
    try {
        const configuration = await controlClient.configuration();
        if (!configuration.enabled) return false;
    } catch {
        return false;
    }

    const nativeInputs = Array.from(form.querySelectorAll<HTMLInputElement>('input[type=file][name^="bf_file"]'));
    const anchor = nativeInputs[0] ?? form.querySelector<HTMLElement>('button[type=submit],input[type=submit]');
    if (!anchor) return false;

    const uploader = documentRoot.createElement('g7mb-g5-uploader') as G5UploaderElement;
    const insertionAnchor = anchor.closest<HTMLElement>('.bo_w_flie, .write_div') ?? anchor;
    insertionAnchor.parentNode?.insertBefore(uploader, insertionAnchor);
    const hidden = documentRoot.createElement('input');
    hidden.type = 'hidden';
    hidden.name = 'g7mb_upload_ids';
    hidden.value = '';
    form.append(hidden);
    for (const input of nativeInputs) {
        input.disabled = true;
        input.hidden = true;
        const nativeWrapper = input.closest<HTMLElement>('.file_wr');
        if (nativeWrapper) nativeWrapper.hidden = true;
        const label = input.id
            ? Array.from(form.querySelectorAll<HTMLLabelElement>('label[for]')).find((candidate) => candidate.htmlFor === input.id) ?? null
            : null;
        if (label) label.hidden = true;
    }

    let running = false;
    let pending = false;
    uploader.addEventListener('g7mb:selection', () => {
        const superseded = parseIds(hidden.value);
        hidden.value = '';
        pending = true;
        // A new selection replaces the visible batch. Remove no-longer-visible
        // Ready uploads on a best-effort basis; server retention is the fallback.
        void Promise.allSettled(superseded.map((uploadId) => controlClient.deleteUpload(uploadId)));
    });
    uploader.addEventListener('g7mb:state', (event) => {
        running = Boolean((event as CustomEvent<{ running: boolean }>).detail?.running);
    });
    uploader.addEventListener('g7mb:complete', (event) => {
        const result = (event as CustomEvent<UploadBatchResult>).detail;
        const current = parseIds(hidden.value);
        const uploaded = result.files
            .filter((file) => file.state === 'accepted' && typeof file.uploadId === 'string')
            .map((file) => file.uploadId as string);
        hidden.value = [...new Set([...current, ...uploaded])].slice(0, 100).join(',');
        pending = false;
    });
    form.addEventListener('submit', (event) => {
        if (!running && !pending) return;
        event.preventDefault();
        event.stopImmediatePropagation();
        window.alert(running ? '미디어 업로드가 끝난 뒤 저장해 주십시오.' : '선택한 파일의 업로드를 먼저 완료해 주십시오.');
    }, true);
    form.dataset.g7mbMounted = 'true';

    return true;
}

function findWriteForm(documentRoot: Document): HTMLFormElement | null {
    return Array.from(documentRoot.forms).find((form) => {
        try {
            const action = new URL(form.action, window.location.origin);
            return action.origin === window.location.origin && action.pathname.endsWith('/bbs/write_update.php');
        } catch {
            return false;
        }
    }) ?? null;
}

function parseIds(value: string): string[] {
    return value === ''
        ? []
        : value.split(',').filter((entry) => /^[a-f0-9-]{36}$/.test(entry));
}
