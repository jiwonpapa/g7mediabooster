interface Attachment {
    id: number;
    original_filename: string;
    preview_url?: string | null;
}

interface NativeApi {
    delete(url: string): Promise<unknown>;
    patch(url: string, data: unknown): Promise<unknown>;
}

/** Native G7 owns authorization, deletion retention and persistent ordering. */
export class ExistingAttachments {
    public readonly element = document.createElement('section');
    private readonly list = document.createElement('ol');
    private readonly status = document.createElement('p');
    private files: Attachment[];
    private busy = false;
    private uploading = false;
    private readonly attachmentPrefix: string;

    public constructor(
        boardSlug: string,
        files: unknown,
        private readonly endpoints: { deleteEndpoint: string; reorderEndpoint: string },
        private readonly disabled: boolean,
        private readonly changed: (ids: number[]) => void,
        private readonly running: (busy: boolean) => void,
        private readonly api?: NativeApi,
    ) {
        if (!/^[A-Za-z0-9_-]+$/.test(boardSlug)) throw new Error('invalid attachment board');
        const allowed = ['boards', 'admin/board'].map(scope => `/api/modules/sirsoft-board/${scope}/${boardSlug}/attachments`);
        if (!allowed.some(base => endpoints.deleteEndpoint === base+'/:id' && endpoints.reorderEndpoint === base+'/reorder')) {
            throw new Error('invalid native attachment endpoint');
        }
        this.attachmentPrefix = `/api/modules/sirsoft-board/boards/${boardSlug}/attachment/`;
        this.files = normalize(files);
        this.element.setAttribute('aria-label', '첨부파일 관리');
        this.element.style.cssText = 'padding:16px 20px;border:1px solid var(--g7-border,#d8dee9);margin-bottom:16px';
        this.list.style.cssText = 'list-style:none;padding:0;margin:0';
        const heading = document.createElement('h3');
        heading.textContent = '준비된 첨부파일';
        this.status.setAttribute('aria-live', 'polite');
        this.element.append(heading, this.list, this.status);
        this.render();
    }

    public ids(): number[] { return this.files.map(file => file.id); }

    public add(files: unknown): void {
        this.files = normalize([...this.files, ...normalize(files)]);
        this.render();
    }

    public setUploading(uploading: boolean): void {
        this.uploading = uploading;
        this.render();
    }

    public async remove(id: number): Promise<void> {
        if (this.locked() || !this.files.some(file => file.id === id)) return;
        if (!window.confirm('이 첨부파일을 삭제하시겠습니까?')) return;
        await this.mutate(async () => {
            assertSuccess(await this.nativeApi().delete(this.endpoints.deleteEndpoint.replace(':id', String(id))));
            this.files = this.files.filter(file => file.id !== id);
        });
    }

    public async move(id: number, direction: -1 | 1): Promise<void> {
        const index = this.files.findIndex(file => file.id === id);
        const nextIndex = index + direction;
        if (this.locked() || index < 0 || nextIndex < 0 || nextIndex >= this.files.length) return;
        const next = [...this.files];
        [next[index], next[nextIndex]] = [next[nextIndex] as Attachment, next[index] as Attachment];
        await this.mutate(async () => {
            assertSuccess(await this.nativeApi().patch(this.endpoints.reorderEndpoint, {
                order: next.map((file, order) => ({ id: file.id, order: order + 1 })),
            }));
            this.files = next;
        });
    }

    private async mutate(action: () => Promise<void>): Promise<void> {
        this.busy = true;
        this.running(true);
        this.render();
        try {
            await action();
            this.changed(this.ids());
            this.status.textContent = '첨부파일을 변경했습니다.';
        } catch {
            this.status.textContent = '첨부파일을 변경하지 못했습니다. 권한과 연결을 확인해 주세요.';
        } finally {
            this.busy = false;
            this.running(false);
            this.render();
        }
    }

    private nativeApi(): NativeApi {
        const api = this.api ?? (window as unknown as { G7Core?: { api?: NativeApi } }).G7Core?.api;
        if (!api) throw new Error('native attachment API unavailable');
        return api;
    }

    private locked(): boolean { return this.disabled || this.busy || this.uploading; }

    private render(): void {
        this.list.replaceChildren();
        this.element.hidden = this.files.length === 0;
        this.files.forEach((file, index) => {
            const row = document.createElement('li');
            row.style.cssText = 'display:flex;align-items:center;flex-wrap:wrap;gap:8px;padding:10px 0';
            row.dataset.attachmentId = String(file.id);
            const preview = this.previewUrl(file.preview_url);
            if (preview) {
                const image = document.createElement('img');
                image.src = preview;
                image.alt = '';
                image.loading = 'lazy';
                image.width = 64;
                image.height = 64;
                image.style.objectFit = 'contain';
                row.append(image);
            }
            const name = document.createElement('span');
            name.style.cssText = 'flex:1 1 160px;overflow-wrap:anywhere';
            name.textContent = file.original_filename;
            row.append(name);
            for (const [label, blocked, action] of [
                ['앞으로', index === 0, () => this.move(file.id, -1)],
                ['뒤로', index === this.files.length - 1, () => this.move(file.id, 1)],
                ['삭제', false, () => this.remove(file.id)],
            ] as const) {
                const button = document.createElement('button');
                button.type = 'button';
                button.style.cssText = 'min-height:36px;padding:6px 12px;border:1px solid var(--g7-border,#d8dee9);background:var(--g7-panel,#fff);color:inherit';
                button.textContent = label;
                button.setAttribute('aria-label', `${file.original_filename} ${label}`);
                button.disabled = blocked || this.locked();
                button.addEventListener('click', () => { void action(); });
                row.append(button);
            }
            this.list.append(row);
        });
    }

    private previewUrl(value: unknown): string | null {
        if (typeof value !== 'string') return null;
        try {
            const url = new URL(value, window.location.origin);
            const suffix = url.pathname.slice(this.attachmentPrefix.length);
            return url.origin === window.location.origin && url.pathname.startsWith(this.attachmentPrefix)
                && /^[A-Za-z0-9]{12}\/preview$/.test(suffix) ? url.href : null;
        } catch { return null; }
    }
}

function normalize(files: unknown): Attachment[] {
    if (!Array.isArray(files) || files.length > 200) throw new Error('invalid attachment list');
    const unique = new Map<number, Attachment>();
    for (const file of files) {
        if (!file || !Number.isSafeInteger(file.id) || file.id < 1
            || typeof file.original_filename !== 'string' || file.original_filename.length > 255) {
            throw new Error('invalid attachment');
        }
        unique.set(file.id, { id: file.id, original_filename: file.original_filename, preview_url: file.preview_url });
    }
    if (unique.size > 100) throw new Error('attachment count exceeds limit');
    return [...unique.values()];
}

function assertSuccess(response: unknown): void {
    if (!response || typeof response !== 'object' || !('success' in response) || response.success !== true) {
        throw new Error('native attachment mutation failed');
    }
}
