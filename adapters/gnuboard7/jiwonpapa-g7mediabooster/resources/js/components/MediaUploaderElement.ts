import { G7MediaControlClient } from '../controlClient';
import type { FileUploadProgress, PublicUploaderConfiguration, UploadBatchResult } from '../types';
import { MultiUploader } from '../upload/MultiUploader';
import { XhrUploadTransport } from '../upload/XhrUploadTransport';

const ACCEPTED_TYPES = '.jpg,.jpeg,.png,.gif,.webp,.avif,.heic,.heif,.mp4,.mov,.webm';

export class G7MediaUploaderElement extends HTMLElement {
    private readonly root: ShadowRoot;
    private readonly input: HTMLInputElement;
    private readonly list: HTMLUListElement;
    private readonly status: HTMLParagraphElement;
    private readonly startButton: HTMLButtonElement;
    private readonly cancelButton: HTMLButtonElement;
    private readonly rows = new WeakMap<File, HTMLLIElement>();
    private selectedFiles: File[] = [];
    private activeController: AbortController | null = null;
    private configuration: PublicUploaderConfiguration | null = null;

    public constructor() {
        super();
        this.root = this.attachShadow({ mode: 'open' });
        this.root.innerHTML = template();
        this.input = requiredElement(this.root, 'input[type=file]', HTMLInputElement);
        this.list = requiredElement(this.root, '[data-role=list]', HTMLUListElement);
        this.status = requiredElement(this.root, '[data-role=status]', HTMLParagraphElement);
        this.startButton = requiredElement(this.root, '[data-role=start]', HTMLButtonElement);
        this.cancelButton = requiredElement(this.root, '[data-role=cancel]', HTMLButtonElement);
    }

    public connectedCallback(): void {
        this.input.accept = ACCEPTED_TYPES;
        this.input.multiple = true;
        this.input.addEventListener('change', this.handleSelection);
        this.startButton.addEventListener('click', this.handleStart);
        this.cancelButton.addEventListener('click', this.handleCancel);
        const dropZone = requiredElement(this.root, '[data-role=dropzone]', HTMLElement);
        dropZone.addEventListener('dragover', this.handleDragOver);
        dropZone.addEventListener('drop', this.handleDrop);
        void this.loadConfiguration();
    }

    public disconnectedCallback(): void {
        this.activeController?.abort(new DOMException('Uploader removed', 'AbortError'));
        this.input.removeEventListener('change', this.handleSelection);
        this.startButton.removeEventListener('click', this.handleStart);
        this.cancelButton.removeEventListener('click', this.handleCancel);
    }

    public async start(files: Iterable<File>): Promise<UploadBatchResult> {
        const boardSlug = this.boardSlug();
        const configuration = this.configuration ?? await this.loadConfiguration();
        if (!configuration.enabled) {
            throw new Error('미디어 부스터가 비활성화되어 있습니다.');
        }
        const selected = Array.from(files);
        validateSelection(selected, configuration);
        this.renderFiles(selected);
        this.activeController = new AbortController();
        this.setRunning(true);
        this.setStatus(`${selected.length}개 파일 업로드를 시작합니다.`);

        try {
            const result = await new MultiUploader(
                new G7MediaControlClient(boardSlug),
                new XhrUploadTransport(),
            ).upload(selected, {
                maxParallelFiles: configuration.max_parallel_files,
                maxParallelParts: configuration.max_parallel_parts,
                maxConnections: configuration.max_parallel_files,
                maxRetries: configuration.max_part_retries,
                signal: this.activeController.signal,
                onProgress: (progress) => this.updateProgress(progress),
            });
            const accepted = result.files.filter((file) => file.state === 'accepted').length;
            const failed = result.files.length - accepted;
            this.setStatus(failed === 0
                ? `${accepted}개 파일을 안전 검사 대기열에 등록했습니다.`
                : `${accepted}개 완료, ${failed}개 실패 또는 취소되었습니다.`);
            this.dispatchEvent(new CustomEvent<UploadBatchResult>('g7mb:complete', {
                detail: result,
                bubbles: true,
                composed: true,
            }));
            return result;
        } finally {
            this.activeController = null;
            this.setRunning(false);
        }
    }

    private readonly handleSelection = (): void => {
        this.selectFiles(Array.from(this.input.files ?? []));
    };

    private readonly handleStart = (): void => {
        void this.start(this.selectedFiles).catch((error: unknown) => {
            this.setStatus(error instanceof Error ? error.message : '업로드를 시작하지 못했습니다.', true);
            this.setRunning(false);
        });
    };

    private readonly handleCancel = (): void => {
        this.activeController?.abort(new DOMException('User cancelled upload', 'AbortError'));
        this.setStatus('업로드 취소를 처리하고 있습니다.');
    };

    private readonly handleDragOver = (event: DragEvent): void => {
        event.preventDefault();
        if (event.dataTransfer) event.dataTransfer.dropEffect = 'copy';
    };

    private readonly handleDrop = (event: DragEvent): void => {
        event.preventDefault();
        this.selectFiles(Array.from(event.dataTransfer?.files ?? []));
    };

    private selectFiles(files: File[]): void {
        try {
            if (this.configuration) validateSelection(files, this.configuration);
            this.selectedFiles = files;
            this.renderFiles(files);
            this.startButton.disabled = files.length === 0 || Boolean(this.activeController);
            this.setStatus(files.length > 0 ? `${files.length}개 파일을 선택했습니다.` : '파일을 선택해 주세요.');
        } catch (error) {
            this.selectedFiles = [];
            this.renderFiles([]);
            this.startButton.disabled = true;
            this.setStatus(error instanceof Error ? error.message : '파일 선택이 올바르지 않습니다.', true);
        }
    }

    private async loadConfiguration(): Promise<PublicUploaderConfiguration> {
        try {
            const configuration = await new G7MediaControlClient(this.boardSlug()).configuration();
            this.configuration = configuration;
            this.input.disabled = !configuration.enabled;
            this.startButton.disabled = !configuration.enabled || this.selectedFiles.length === 0;
            this.setStatus(configuration.enabled
                ? `최대 ${configuration.max_files}개, 파일당 ${formatBytes(configuration.max_file_size_bytes)}까지 업로드할 수 있습니다.`
                : '관리자가 미디어 부스터를 활성화해야 합니다.');
            return configuration;
        } catch (error) {
            this.input.disabled = true;
            this.startButton.disabled = true;
            this.setStatus(error instanceof Error ? error.message : '업로더 설정을 불러오지 못했습니다.', true);
            throw error;
        }
    }

    private renderFiles(files: File[]): void {
        this.list.replaceChildren();
        for (const file of files) {
            const row = document.createElement('li');
            row.className = 'file-row';
            const summary = document.createElement('div');
            summary.className = 'file-summary';
            const name = document.createElement('span');
            name.className = 'file-name';
            name.textContent = file.name;
            const meta = document.createElement('span');
            meta.className = 'file-meta';
            meta.textContent = `${formatBytes(file.size)} · 대기`;
            const progress = document.createElement('progress');
            progress.max = 100;
            progress.value = 0;
            progress.setAttribute('aria-label', `${file.name} 업로드 진행률`);
            summary.append(name, meta);
            row.append(summary, progress);
            this.rows.set(file, row);
            this.list.append(row);
        }
    }

    private updateProgress(progress: FileUploadProgress): void {
        const row = this.rows.get(progress.file);
        if (!row) return;
        const bar = row.querySelector('progress');
        const meta = row.querySelector<HTMLElement>('.file-meta');
        if (bar) bar.value = progress.percent;
        if (meta) meta.textContent = `${formatBytes(progress.file.size)} · ${stateLabel(progress)}`;
        row.dataset.state = progress.state;
    }

    private setRunning(running: boolean): void {
        this.input.disabled = running || this.configuration?.enabled === false;
        this.startButton.disabled = running || this.selectedFiles.length === 0;
        this.cancelButton.hidden = !running;
    }

    private setStatus(message: string, error = false): void {
        this.status.textContent = message;
        this.status.dataset.error = String(error);
    }

    private boardSlug(): string {
        const slug = this.getAttribute('board-slug')?.trim() ?? '';
        if (!/^[A-Za-z0-9_-]+$/.test(slug)) {
            throw new Error('board-slug 속성이 필요합니다.');
        }
        return slug;
    }
}

function validateSelection(files: File[], configuration: PublicUploaderConfiguration): void {
    if (files.length < 1) throw new RangeError('한 개 이상의 파일을 선택해 주세요.');
    if (files.length > Math.min(100, configuration.max_files)) {
        throw new RangeError(`한 번에 최대 ${Math.min(100, configuration.max_files)}개까지 선택할 수 있습니다.`);
    }
    const oversized = files.find((file) => file.size > configuration.max_file_size_bytes);
    if (oversized) throw new RangeError(`${oversized.name}: 게시판 파일 크기 제한을 초과했습니다.`);
}

function stateLabel(progress: FileUploadProgress): string {
    switch (progress.state) {
        case 'queued': return '대기';
        case 'uploading': return `${progress.percent}%`;
        case 'verifying': return '확인 중';
        case 'accepted': return '검사 대기';
        case 'cancelled': return '취소됨';
        case 'failed': return progress.error ?? '실패';
    }
}

function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KiB', 'MiB', 'GiB'];
    let value = bytes / 1024;
    let unit = units[0] as string;
    for (let index = 1; index < units.length && value >= 1024; index += 1) {
        value /= 1024;
        unit = units[index] as string;
    }
    return `${value.toFixed(value >= 10 ? 0 : 1)} ${unit}`;
}

function requiredElement<T extends Element>(root: ParentNode, selector: string, constructor: { new (): T }): T {
    const element = root.querySelector(selector);
    if (!(element instanceof constructor)) throw new Error(`missing uploader element: ${selector}`);
    return element;
}

function template(): string {
    return `
        <style>
            :host { display: block; color: var(--g7-text, #172033); font: 14px/1.5 system-ui, sans-serif; }
            .shell { border: 1px solid var(--g7-border, #d8dee9); background: var(--g7-panel, #fff); }
            .header { padding: 18px 20px 14px; border-bottom: 1px solid var(--g7-border, #d8dee9); }
            h2 { margin: 0 0 4px; font-size: 17px; }
            p { margin: 0; color: var(--g7-muted, #64748b); }
            .dropzone { margin: 16px 20px; padding: 24px; border: 1px dashed #8a98ad; text-align: center; background: #f8fafc; }
            input { max-width: 100%; }
            ul { margin: 0; padding: 0 20px; list-style: none; max-height: 320px; overflow: auto; }
            .file-row { padding: 11px 0; border-top: 1px solid #edf0f4; }
            .file-summary { display: flex; justify-content: space-between; gap: 14px; margin-bottom: 7px; }
            .file-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
            .file-meta { flex: none; color: #64748b; font-size: 12px; }
            .file-row[data-state='failed'] .file-meta { color: #b42318; }
            progress { display: block; width: 100%; height: 7px; accent-color: #2255d6; }
            .actions { display: flex; align-items: center; gap: 10px; padding: 16px 20px 20px; }
            button { min-height: 40px; padding: 8px 16px; border: 1px solid #2255d6; background: #2255d6; color: white; cursor: pointer; }
            button.secondary { border-color: #9aa6b6; background: white; color: #263248; }
            button:disabled { cursor: not-allowed; opacity: .5; }
            [data-role='status'] { flex: 1; font-size: 13px; }
            [data-role='status'][data-error='true'] { color: #b42318; }
            @media (max-width: 600px) {
                .header, .actions { padding-left: 14px; padding-right: 14px; }
                .dropzone { margin-left: 14px; margin-right: 14px; }
                ul { padding: 0 14px; }
                .actions { align-items: stretch; flex-direction: column; }
                button { width: 100%; }
            }
        </style>
        <section class="shell" aria-labelledby="g7mb-title">
            <header class="header">
                <h2 id="g7mb-title">미디어 업로드</h2>
                <p>파일은 PHP 서버를 거치지 않고 저장소로 직접 전송됩니다.</p>
            </header>
            <label class="dropzone" data-role="dropzone">
                <span>이미지·동영상을 놓거나 파일을 선택하세요.</span><br><br>
                <input type="file" aria-describedby="g7mb-status">
            </label>
            <ul data-role="list" aria-label="업로드 파일"></ul>
            <div class="actions">
                <p id="g7mb-status" data-role="status" aria-live="polite">업로더 설정을 확인하고 있습니다.</p>
                <button type="button" data-role="cancel" class="secondary" hidden>전체 취소</button>
                <button type="button" data-role="start" disabled>업로드 시작</button>
            </div>
        </section>`;
}
