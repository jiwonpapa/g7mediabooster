const MODULE_IDENTIFIER = 'jiwonpapa-g7mediabooster';
const HANDLER_NAME = `${MODULE_IDENTIFIER}.mountUploader`;
const ELEMENT_NAME = 'g7-media-uploader';
const MAX_ATTACHMENT_IDS = 100;

interface ActionDefinition {
    params?: Record<string, unknown>;
}

interface G7StateBridge {
    getLocal?: () => unknown;
    setLocal?: (
        updates: Record<string, unknown>,
        options?: { merge?: 'deep'; render?: boolean },
    ) => void;
}

interface G7ActionDispatcher {
    registerHandler: (
        name: string,
        handler: (action: ActionDefinition) => void,
        options: { category: 'module'; source: string },
    ) => void;
}

interface G7Runtime {
    getActionDispatcher?: () => G7ActionDispatcher | undefined;
    state?: G7StateBridge;
}

interface CompletedUploadBatchEvent {
    batchId: string;
    files: Array<{
        state: string;
        attachment: { id: number } | null;
    }>;
}

type G7Window = Window & { G7Core?: G7Runtime };

export function mountUploaderHandler(action: ActionDefinition): void {
    const mountId = action.params?.mountId;
    const boardSlug = action.params?.boardSlug;
    if (typeof mountId !== 'string' || !/^[A-Za-z][A-Za-z0-9_-]{0,79}$/.test(mountId)) {
        throw new Error('invalid G7 Media Booster mount id');
    }
    if (typeof boardSlug !== 'string' || !/^[A-Za-z0-9_-]+$/.test(boardSlug)) {
        throw new Error('invalid G7 Media Booster board slug');
    }

    const mount = document.getElementById(mountId);
    if (!(mount instanceof HTMLElement)) {
        throw new Error('G7 Media Booster mount element is missing');
    }
    if (mount.dataset.g7mbMounted === 'true') {
        return;
    }

    updateLocalState({ g7mbUploading: false });
    const uploader = document.createElement(ELEMENT_NAME);
    uploader.setAttribute('board-slug', boardSlug);
    uploader.addEventListener('g7mb:state', handleUploaderState);
    uploader.addEventListener('g7mb:complete', handleUploaderComplete);
    mount.replaceChildren(uploader);
    mount.dataset.g7mbMounted = 'true';
}

export function registerFormBridge(retry = false): void {
    const runtime = (window as unknown as G7Window).G7Core;
    const dispatcher = runtime?.getActionDispatcher?.();
    if (dispatcher) {
        dispatcher.registerHandler(HANDLER_NAME, mountUploaderHandler, {
            category: 'module',
            source: MODULE_IDENTIFIER,
        });
        return;
    }
    if (!retry) return;

    let attempts = 0;
    const retryRegister = (): void => {
        attempts += 1;
        const next = (window as unknown as G7Window).G7Core?.getActionDispatcher?.();
        if (next) {
            next.registerHandler(HANDLER_NAME, mountUploaderHandler, {
                category: 'module',
                source: MODULE_IDENTIFIER,
            });
            return;
        }
        if (attempts < 50) window.setTimeout(retryRegister, 100);
    };
    window.setTimeout(retryRegister, 100);
}

function handleUploaderState(event: Event): void {
    if (!(event instanceof CustomEvent) || !isRecord(event.detail) || typeof event.detail.running !== 'boolean') {
        return;
    }
    updateLocalState({ g7mbUploading: event.detail.running });
}

function handleUploaderComplete(event: Event): void {
    if (!(event instanceof CustomEvent) || !isUploadBatchResult(event.detail)) {
        return;
    }
    const addedIds = event.detail.files
        .filter((file) => file.state === 'accepted' && file.attachment !== null)
        .map((file) => file.attachment?.id)
        .filter((id): id is number => Number.isSafeInteger(id) && (id ?? 0) > 0);
    if (addedIds.length === 0) {
        updateLocalState({ g7mbUploading: false });
        return;
    }

    const current = currentLocalState();
    const form = isRecord(current.form) ? current.form : {};
    const existingIds = Array.isArray(form.attachment_ids)
        ? form.attachment_ids.filter((id): id is number => Number.isSafeInteger(id) && (id ?? 0) > 0)
        : [];
    const attachmentIds = [...new Set([...existingIds, ...addedIds])];
    if (attachmentIds.length > MAX_ATTACHMENT_IDS) {
        throw new RangeError('G7 attachment count exceeds the supported maximum');
    }

    updateLocalState({
        'form.attachment_ids': attachmentIds,
        g7mbUploading: false,
        hasChanges: true,
    });
}

function currentLocalState(): Record<string, unknown> {
    const current = (window as unknown as G7Window).G7Core?.state?.getLocal?.();
    return isRecord(current) ? current : {};
}

function updateLocalState(updates: Record<string, unknown>): void {
    const state = (window as unknown as G7Window).G7Core?.state;
    if (typeof state?.setLocal !== 'function') {
        throw new Error('G7 local state bridge is unavailable');
    }
    state.setLocal(updates, { merge: 'deep', render: true });
}

function isUploadBatchResult(value: unknown): value is CompletedUploadBatchEvent {
    return isRecord(value)
        && typeof value.batchId === 'string'
        && Array.isArray(value.files)
        && value.files.every((file) => isRecord(file)
            && typeof file.state === 'string'
            && (file.attachment === null
                || (isRecord(file.attachment)
                    && Number.isSafeInteger(file.attachment.id)
                    && (file.attachment.id as number) > 0)));
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
