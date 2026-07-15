import { mountG5Uploader } from './FormBridge';
import { G5UploaderElement } from './G5UploaderElement';

if (!customElements.get('g7mb-g5-uploader')) {
    customElements.define('g7mb-g5-uploader', G5UploaderElement);
}

const mount = (): void => { void mountG5Uploader(); };
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', mount, { once: true });
} else {
    mount();
}

export { G5MediaControlClient } from './controlClient';
export { G5UploaderElement } from './G5UploaderElement';
export { mountG5Uploader } from './FormBridge';
