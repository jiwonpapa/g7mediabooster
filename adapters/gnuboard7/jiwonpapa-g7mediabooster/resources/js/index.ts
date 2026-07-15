import { G7MediaUploaderElement } from './components/MediaUploaderElement';
import { G7MediaControlClient } from './controlClient';
import { MultiUploader } from './upload/MultiUploader';
import { XhrUploadTransport } from './upload/XhrUploadTransport';

const ELEMENT_NAME = 'g7-media-uploader';

if (!customElements.get(ELEMENT_NAME)) {
    customElements.define(ELEMENT_NAME, G7MediaUploaderElement);
}

const api = {
    elementName: ELEMENT_NAME,
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

export { G7MediaControlClient, G7MediaUploaderElement, MultiUploader, XhrUploadTransport };
