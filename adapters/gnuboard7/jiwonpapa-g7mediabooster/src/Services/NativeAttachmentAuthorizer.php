<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use Illuminate\Http\Request;
use Modules\Sirsoft\Board\Models\Attachment;
use Modules\Sirsoft\Board\Repositories\Contracts\AttachmentRepositoryInterface;
use Modules\Sirsoft\Board\Repositories\Contracts\BoardRepositoryInterface;
use Modules\Sirsoft\Board\Services\AttachmentService;
use Symfony\Component\HttpKernel\Exception\AccessDeniedHttpException;

/** Reuses stock board scope/secret/blind/deleted gates with an I/O-denying storage. */
final class NativeAttachmentAuthorizer
{
    public function __construct(
        private readonly AttachmentRepositoryInterface $attachments,
        private readonly BoardRepositoryInterface $boards,
    ) {}

    public function authorize(Request $request, string $slug, Attachment $attachment, string $variant, string $context = 'user'): void
    {
        $native = new AttachmentService($this->attachments, $this->boards, new AuthorizationStorage((string) $attachment->path));
        $authorized = $variant === 'master'
            ? $native->download($slug, (int) $attachment->id, $context)
            : $native->getFileInfo($slug, (int) $attachment->id, $request->hasValidSignature(absolute: false));
        if ($authorized === null) {
            throw new AccessDeniedHttpException('G7MB_ATTACHMENT_NOT_AUTHORIZED');
        }
    }
}
