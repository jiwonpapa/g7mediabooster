const ENDPOINT = '/api/modules/jiwonpapa-g7mediabooster/admin/watermark-assets';
const MAX_SOURCE_BYTES = 16 * 1024 * 1024;
const MAX_ASSETS = 50;
const UUID_PATTERN = /^[a-f0-9]{8}-[a-f0-9]{4}-[1-8][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/;

interface WatermarkAsset {
    upload_id: string;
    filename: string;
    source_bytes: number;
    detected_content_type: 'image/jpeg' | 'image/png' | 'image/webp';
    board_slug: string;
    created_at: string;
}

interface WatermarkAssetResponse {
    assets: WatermarkAsset[];
    selected_upload_id: string;
}

interface G7Api {
    get<T>(url: string): Promise<T>;
}

type AdminWindow = Window & { G7Core?: { api?: G7Api } };

export class G7WatermarkAssetPickerElement extends HTMLElement {
    public static readonly observedAttributes = ['selected-upload-id'];

    private readonly root: ShadowRoot;
    private readonly list: HTMLDivElement;
    private readonly status: HTMLParagraphElement;
    private readonly refreshButton: HTMLButtonElement;
    private readonly clearButton: HTMLButtonElement;
    private assets: WatermarkAsset[] = [];
    private selectedId = '';

    public constructor() {
        super();
        this.root = this.attachShadow({ mode: 'open' });
        this.root.innerHTML = template();
        this.list = requiredElement(this.root, '[data-role=list]', HTMLDivElement);
        this.status = requiredElement(this.root, '[data-role=status]', HTMLParagraphElement);
        this.refreshButton = requiredElement(this.root, '[data-role=refresh]', HTMLButtonElement);
        this.clearButton = requiredElement(this.root, '[data-role=clear]', HTMLButtonElement);
    }

    public connectedCallback(): void {
        this.selectedId = normalizedUploadId(this.getAttribute('selected-upload-id'));
        this.refreshButton.addEventListener('click', this.handleRefresh);
        this.clearButton.addEventListener('click', this.handleClear);
        void this.refresh();
    }

    public disconnectedCallback(): void {
        this.refreshButton.removeEventListener('click', this.handleRefresh);
        this.clearButton.removeEventListener('click', this.handleClear);
    }

    public attributeChangedCallback(name: string, _oldValue: string | null, newValue: string | null): void {
        if (name !== 'selected-upload-id') return;
        this.selectedId = normalizedUploadId(newValue);
        this.render();
    }

    private readonly handleRefresh = (): void => {
        void this.refresh();
    };

    private readonly handleClear = (): void => {
        this.select('');
    };

    private async refresh(): Promise<void> {
        this.refreshButton.disabled = true;
        this.setStatus('검증 완료된 워터마크 이미지를 불러오는 중입니다.');
        try {
            const api = (window as unknown as AdminWindow).G7Core?.api;
            if (!api) throw new Error('G7Core.api is not available');
            const response = validateResponse(await api.get<unknown>(ENDPOINT));
            this.assets = response.assets;
            if (this.selectedId === '') this.selectedId = response.selected_upload_id;
            this.render();
        } catch {
            this.assets = [];
            this.list.replaceChildren();
            this.setStatus('워터마크 자산을 불러오지 못했습니다.', true);
        } finally {
            this.refreshButton.disabled = false;
        }
    }

    private render(): void {
        this.list.replaceChildren();
        for (const asset of this.assets) {
            const label = document.createElement('label');
            label.className = 'asset';

            const radio = document.createElement('input');
            radio.type = 'radio';
            radio.name = 'g7mb-watermark-asset';
            radio.value = asset.upload_id;
            radio.checked = asset.upload_id === this.selectedId;
            radio.addEventListener('change', () => {
                if (radio.checked) this.select(asset.upload_id);
            });

            const preview = document.createElement('span');
            preview.className = 'preview';
            preview.textContent = asset.detected_content_type.split('/')[1]?.toUpperCase() ?? 'IMG';

            const copy = document.createElement('span');
            copy.className = 'copy';
            const filename = document.createElement('strong');
            filename.textContent = asset.filename;
            const meta = document.createElement('small');
            meta.textContent = `${asset.detected_content_type} · ${formatBytes(asset.source_bytes)}`;
            copy.append(filename, meta);
            label.append(radio, preview, copy);
            this.list.append(label);
        }

        this.clearButton.disabled = this.selectedId === '';
        if (this.assets.length === 0) {
            this.setStatus('최근 7일 안에 직접 업로드한 Ready JPEG·PNG·WebP 이미지가 없습니다.');
        } else if (this.selectedId !== '' && !this.assets.some((asset) => asset.upload_id === this.selectedId)) {
            this.setStatus('현재 정책 자산은 목록 유효기간이 지나 미리보기를 표시할 수 없습니다. 새 자산을 선택하거나 해제해 주세요.', true);
        } else {
            this.setStatus(`${this.assets.length}개 자산 중 하나를 선택할 수 있습니다.`);
        }
    }

    private select(uploadId: string): void {
        this.selectedId = uploadId;
        if (uploadId === '') this.removeAttribute('selected-upload-id');
        else this.setAttribute('selected-upload-id', uploadId);
        this.render();
        this.dispatchEvent(new CustomEvent('g7mb:watermark-selected', {
            detail: { uploadId },
            bubbles: true,
            composed: true,
        }));
    }

    private setStatus(message: string, error = false): void {
        this.status.textContent = message;
        this.status.dataset.error = String(error);
    }
}

function validateResponse(value: unknown): WatermarkAssetResponse {
    if (!isRecord(value) || value.success !== true || !isRecord(value.data)) {
        throw new Error('invalid watermark asset response');
    }
    const assets = value.data.assets;
    const selected = value.data.selected_upload_id;
    if (!Array.isArray(assets)
        || assets.length > MAX_ASSETS
        || typeof selected !== 'string'
        || (selected !== '' && !UUID_PATTERN.test(selected))
    ) {
        throw new Error('invalid watermark asset response');
    }

    return {
        assets: assets.map(validateAsset),
        selected_upload_id: selected,
    };
}

function validateAsset(value: unknown): WatermarkAsset {
    if (!isRecord(value)) throw new Error('invalid watermark asset');
    const uploadId = value.upload_id;
    const filename = value.filename;
    const sourceBytes = value.source_bytes;
    const detectedType = value.detected_content_type;
    const boardSlug = value.board_slug;
    const createdAt = value.created_at;
    if (typeof uploadId !== 'string'
        || !UUID_PATTERN.test(uploadId)
        || typeof filename !== 'string'
        || filename.length < 1
        || filename.length > 255
        || /[\u0000-\u001f\u007f/\\]/u.test(filename)
        || !Number.isSafeInteger(sourceBytes)
        || (sourceBytes as number) < 1
        || (sourceBytes as number) > MAX_SOURCE_BYTES
        || !['image/jpeg', 'image/png', 'image/webp'].includes(String(detectedType))
        || typeof boardSlug !== 'string'
        || !/^[A-Za-z0-9_-]{1,100}$/.test(boardSlug)
        || typeof createdAt !== 'string'
        || createdAt.length > 64
    ) {
        throw new Error('invalid watermark asset');
    }

    return value as unknown as WatermarkAsset;
}

function normalizedUploadId(value: string | null): string {
    const normalized = value?.trim().toLowerCase() ?? '';
    return UUID_PATTERN.test(normalized) ? normalized : '';
}

function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function requiredElement<T extends Element>(root: ParentNode, selector: string, constructor: { new (): T }): T {
    const element = root.querySelector(selector);
    if (!(element instanceof constructor)) throw new Error(`missing watermark picker element: ${selector}`);
    return element;
}

function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function template(): string {
    return `
        <style>
            :host { display: block; color: var(--g7-text, #172033); font: 14px/1.5 system-ui, sans-serif; }
            .toolbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 10px; }
            p { margin: 0; color: var(--g7-muted, #64748b); }
            p[data-error=true] { color: #b42318; }
            .actions { display: flex; gap: 8px; }
            button { border: 1px solid var(--g7-border, #d8dee9); background: var(--g7-panel, #fff); padding: 6px 10px; cursor: pointer; }
            button:disabled { cursor: not-allowed; opacity: .55; }
            .list { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 10px; max-height: 360px; overflow: auto; }
            .asset { display: grid; grid-template-columns: auto 64px 1fr; align-items: center; gap: 10px; padding: 10px; border: 1px solid var(--g7-border, #d8dee9); background: var(--g7-panel, #fff); cursor: pointer; }
            .asset:has(input:checked) { border-color: #2563eb; box-shadow: inset 0 0 0 1px #2563eb; }
            .preview { display: grid; place-items: center; width: 64px; height: 64px; background: #f1f5f9; color: #475569; font-size: 11px; font-weight: 700; }
            .copy { min-width: 0; display: grid; gap: 3px; }
            strong, small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
            small { color: var(--g7-muted, #64748b); }
        </style>
        <div class="toolbar">
            <p data-role="status" aria-live="polite"></p>
            <div class="actions">
                <button type="button" data-role="refresh">새로고침</button>
                <button type="button" data-role="clear">선택 해제</button>
            </div>
        </div>
        <div class="list" data-role="list"></div>
    `;
}
