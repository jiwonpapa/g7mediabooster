import type {
    FileUploadProgress,
    PublicUploaderConfiguration,
    UploadBatchResult,
} from '../../../../gnuboard7/jiwonpapa-g7mediabooster/resources/js/types';
import { MultiUploader } from '../../../../gnuboard7/jiwonpapa-g7mediabooster/resources/js/upload/MultiUploader';
import { XhrUploadTransport } from '../../../../gnuboard7/jiwonpapa-g7mediabooster/resources/js/upload/XhrUploadTransport';
import { G5MediaControlClient } from './controlClient';

const ACCEPTED_TYPES = '.jpg,.jpeg,.png,.gif,.webp,.avif,.heic,.heif,.mp4';

export class G5UploaderElement extends HTMLElement {
    private readonly root = this.attachShadow({ mode: 'open' });
    private readonly input: HTMLInputElement;
    private readonly status: HTMLParagraphElement;
    private readonly startButton: HTMLButtonElement;
    private readonly cancelButton: HTMLButtonElement;
    private readonly list: HTMLUListElement;
    private readonly rows = new WeakMap<File, HTMLLIElement>();
    private selected: File[] = [];
    private controller: AbortController | null = null;
    private configuration: PublicUploaderConfiguration | null = null;

    public constructor() {
        super();
        this.root.innerHTML = template();
        this.input = required(this.root, 'input[type=file]', HTMLInputElement);
        this.status = required(this.root, '[data-role=status]', HTMLParagraphElement);
        this.startButton = required(this.root, '[data-role=start]', HTMLButtonElement);
        this.cancelButton = required(this.root, '[data-role=cancel]', HTMLButtonElement);
        this.list = required(this.root, '[data-role=list]', HTMLUListElement);
    }

    public connectedCallback(): void {
        this.input.accept = ACCEPTED_TYPES;
        this.input.multiple = true;
        this.input.addEventListener('change', this.onSelection);
        this.startButton.addEventListener('click', this.onStart);
        this.cancelButton.addEventListener('click', this.onCancel);
        void this.loadConfiguration();
    }

    public disconnectedCallback(): void {
        this.controller?.abort(new DOMException('Uploader removed', 'AbortError'));
        this.input.removeEventListener('change', this.onSelection);
        this.startButton.removeEventListener('click', this.onStart);
        this.cancelButton.removeEventListener('click', this.onCancel);
    }

    public hasPendingSelection(): boolean {
        return this.selected.length > 0;
    }

    public isRunning(): boolean {
        return this.controller !== null;
    }

    private readonly onSelection = (): void => {
        const files = Array.from(this.input.files ?? []);
        try {
            if (!this.configuration) throw new Error('업로더 설정을 확인하고 있습니다.');
            validateSelection(files, this.configuration);
            this.selected = files;
            this.render(files);
            this.startButton.disabled = false;
            this.setStatus(`${files.length}개 파일을 선택했습니다.`);
            this.dispatchEvent(new CustomEvent('g7mb:selection', { bubbles: true, composed: true }));
        } catch (error) {
            this.selected = [];
            this.render([]);
            this.startButton.disabled = true;
            this.setStatus(error instanceof Error ? error.message : '파일 선택이 올바르지 않습니다.', true);
        }
    };

    private readonly onStart = (): void => {
        void this.upload().catch((error: unknown) => {
            this.setStatus(error instanceof Error ? error.message : '업로드를 시작하지 못했습니다.', true);
            this.setRunning(false);
        });
    };

    private readonly onCancel = (): void => {
        this.controller?.abort(new DOMException('User cancelled upload', 'AbortError'));
    };

    private async upload(): Promise<void> {
        const configuration = this.configuration;
        if (!configuration) throw new Error('업로더 설정을 불러오지 못했습니다.');
        validateSelection(this.selected, configuration);
        this.controller = new AbortController();
        this.setRunning(true);
        try {
            const result = await new MultiUploader(
                new G5MediaControlClient(),
                new XhrUploadTransport(),
            ).upload(this.selected, {
                maxParallelFiles: configuration.max_parallel_files,
                maxParallelParts: configuration.max_parallel_parts,
                maxConnections: configuration.max_parallel_files,
                maxRetries: configuration.max_part_retries,
                statusPollIntervalMs: configuration.status_poll_interval_ms,
                signal: this.controller.signal,
                onProgress: (progress) => this.progress(progress),
            });
            const accepted = result.files.filter((file) => file.state === 'accepted' && file.uploadId);
            this.setStatus(accepted.length === result.files.length
                ? `${accepted.length}개 파일이 게시글 첨부 준비를 마쳤습니다.`
                : `${accepted.length}개 완료, ${result.files.length - accepted.length}개 실패 또는 취소되었습니다.`);
            this.selected = [];
            this.input.value = '';
            this.dispatchEvent(new CustomEvent<UploadBatchResult>('g7mb:complete', {
                detail: result,
                bubbles: true,
                composed: true,
            }));
        } finally {
            this.controller = null;
            this.setRunning(false);
        }
    }

    private async loadConfiguration(): Promise<void> {
        try {
            const configuration = await new G5MediaControlClient().configuration();
            if (!configuration.enabled) throw new Error('미디어 부스터가 비활성화되어 있습니다.');
            this.configuration = configuration;
            this.input.disabled = false;
            this.setStatus(`최대 ${configuration.max_files}개, 파일당 ${formatBytes(configuration.max_file_size_bytes)}까지 업로드할 수 있습니다.`);
            this.dispatchEvent(new CustomEvent('g7mb:ready', { bubbles: true, composed: true }));
        } catch (error) {
            this.input.disabled = true;
            this.startButton.disabled = true;
            this.setStatus(error instanceof Error ? error.message : '업로더를 사용할 수 없습니다.', true);
            this.dispatchEvent(new CustomEvent('g7mb:unavailable', { bubbles: true, composed: true }));
        }
    }

    private render(files: File[]): void {
        this.list.replaceChildren();
        for (const file of files) {
            const row = document.createElement('li');
            const label = document.createElement('span');
            label.textContent = file.name;
            const meta = document.createElement('span');
            meta.dataset.role = 'meta';
            meta.textContent = `${formatBytes(file.size)} · 대기`;
            const progress = document.createElement('progress');
            progress.max = 100;
            progress.value = 0;
            row.append(label, meta, progress);
            this.rows.set(file, row);
            this.list.append(row);
        }
    }

    private progress(value: FileUploadProgress): void {
        const row = this.rows.get(value.file);
        if (!row) return;
        const bar = row.querySelector('progress');
        const meta = row.querySelector<HTMLElement>('[data-role=meta]');
        if (bar) bar.value = value.percent;
        if (meta) meta.textContent = `${formatBytes(value.file.size)} · ${stateLabel(value)}`;
    }

    private setRunning(running: boolean): void {
        this.input.disabled = running || this.configuration === null;
        this.startButton.disabled = running || this.selected.length === 0;
        this.cancelButton.hidden = !running;
        this.dispatchEvent(new CustomEvent('g7mb:state', {
            detail: { running },
            bubbles: true,
            composed: true,
        }));
    }

    private setStatus(message: string, error = false): void {
        this.status.textContent = message;
        this.status.dataset.error = String(error);
    }
}

function validateSelection(files: File[], configuration: PublicUploaderConfiguration): void {
    if (files.length < 1) throw new RangeError('한 개 이상의 파일을 선택해 주세요.');
    if (files.length > Math.min(100, configuration.max_files)) throw new RangeError('게시판 파일 개수 제한을 초과했습니다.');
    const oversized = files.find((file) => file.size < 1 || file.size > configuration.max_file_size_bytes);
    if (oversized) throw new RangeError(`${oversized.name}: 게시판 파일 크기 제한을 초과했습니다.`);
}

function stateLabel(progress: FileUploadProgress): string {
    if (progress.state === 'uploading') return `${progress.percent}%`;
    if (progress.state === 'accepted') return '첨부 준비 완료';
    if (progress.state === 'verifying') return '안전 검사 중';
    if (progress.state === 'failed') return progress.error ?? '실패';
    if (progress.state === 'cancelled') return '취소됨';
    return '대기';
}

function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KiB', 'MiB', 'GiB'];
    let value = bytes / 1024;
    let index = 0;
    while (value >= 1024 && index < units.length - 1) {
        value /= 1024;
        index++;
    }
    return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[index]}`;
}

function required<T extends Element>(root: ParentNode, selector: string, type: { new(): T }): T {
    const element = root.querySelector(selector);
    if (!(element instanceof type)) throw new Error(`missing uploader element: ${selector}`);
    return element;
}

function template(): string {
    return `<style>
        :host{display:block;margin:16px 0;color:#172033;font:14px/1.5 system-ui,sans-serif}.shell{border:1px solid #d8dee9;background:#fff}
        header{padding:16px 18px;border-bottom:1px solid #d8dee9}h2,p{margin:0}h2{font-size:17px}header p{color:#64748b}
        .pick{display:block;margin:16px;padding:22px;border:1px dashed #8a98ad;text-align:center;background:#f8fafc}
        ul{margin:0;padding:0 18px;list-style:none;max-height:280px;overflow:auto}li{display:grid;grid-template-columns:minmax(0,1fr) auto;gap:6px 12px;padding:10px 0;border-top:1px solid #edf0f4}li span:first-child{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}li span:nth-child(2){font-size:12px;color:#64748b}progress{grid-column:1/-1;width:100%;height:7px}
        .actions{display:flex;align-items:center;gap:10px;padding:16px 18px}.actions p{flex:1;color:#64748b}.actions p[data-error=true]{color:#b42318}
        button{min-height:38px;padding:7px 14px;border:1px solid #2255d6;background:#2255d6;color:#fff}button.secondary{background:#fff;color:#263248;border-color:#9aa6b6}button:disabled{opacity:.5}
    </style><section class="shell"><header><h2>미디어 직접 업로드</h2><p>파일 바이트는 PHP 서버를 거치지 않습니다.</p></header>
    <label class="pick">이미지·MP4 선택<br><br><input type="file" disabled></label><ul data-role="list"></ul>
    <div class="actions"><p data-role="status" aria-live="polite">설정을 확인하고 있습니다.</p><button type="button" class="secondary" data-role="cancel" hidden>취소</button><button type="button" data-role="start" disabled>업로드 시작</button></div></section>`;
}
