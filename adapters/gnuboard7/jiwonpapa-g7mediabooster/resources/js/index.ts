import { G7MediaUploaderElement } from './components/MediaUploaderElement';
import { G7WatermarkAssetPickerElement } from './components/WatermarkAssetPickerElement';
import { G7MediaControlClient } from './controlClient';
import { registerFormBridge } from './integration/FormAttachmentBridge';
import { MultiUploader } from './upload/MultiUploader';
import { XhrUploadTransport } from './upload/XhrUploadTransport';

const ELEMENT_NAME = 'g7-media-uploader';
const WATERMARK_ELEMENT_NAME = 'g7-watermark-asset-picker';

if (!customElements.get(ELEMENT_NAME)) {
    customElements.define(ELEMENT_NAME, G7MediaUploaderElement);
}
if (!customElements.get(WATERMARK_ELEMENT_NAME)) {
    customElements.define(WATERMARK_ELEMENT_NAME, G7WatermarkAssetPickerElement);
}

if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => registerFormBridge(true), { once: true });
} else {
    registerFormBridge(true);
}

const api = {
    elementName: ELEMENT_NAME,
    watermarkElementName: WATERMARK_ELEMENT_NAME,
    createUploader(boardSlug: string): MultiUploader {
        return new MultiUploader(new G7MediaControlClient(boardSlug), new XhrUploadTransport());
    },
};

window.__G7MediaBooster = api;

declare global {
    interface Window {
        __G7MediaBooster: typeof api;
    }
}

export {
    G7MediaControlClient,
    G7MediaUploaderElement,
    G7WatermarkAssetPickerElement,
    MultiUploader,
    XhrUploadTransport,
};
