export class Semaphore {
    private active = 0;
    private readonly waiters: Array<() => void> = [];

    public constructor(private readonly limit: number) {
        if (!Number.isInteger(limit) || limit < 1) {
            throw new RangeError('semaphore limit must be a positive integer');
        }
    }

    public async use<T>(signal: AbortSignal, operation: () => Promise<T>): Promise<T> {
        await this.acquire(signal);
        try {
            return await operation();
        } finally {
            this.release();
        }
    }

    private async acquire(signal: AbortSignal): Promise<void> {
        signal.throwIfAborted();
        if (this.active < this.limit) {
            this.active += 1;
            return;
        }

        await new Promise<void>((resolve, reject) => {
            const wake = (): void => {
                signal.removeEventListener('abort', abort);
                this.active += 1;
                resolve();
            };
            const abort = (): void => {
                const index = this.waiters.indexOf(wake);
                if (index >= 0) {
                    this.waiters.splice(index, 1);
                }
                reject(signal.reason ?? new DOMException('Upload aborted', 'AbortError'));
            };
            this.waiters.push(wake);
            signal.addEventListener('abort', abort, { once: true });
        });
    }

    private release(): void {
        this.active -= 1;
        this.waiters.shift()?.();
    }
}
