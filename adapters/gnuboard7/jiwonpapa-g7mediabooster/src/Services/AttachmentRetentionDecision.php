<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

final class AttachmentRetentionDecision
{
    public const DELETE = 'delete';
    public const CANCEL = 'cancel';
    public const BLOCK = 'block';

    /** @param array<string, mixed>|null $attachment */
    public function evaluate(?array $attachment, string $uploadId): string
    {
        if ($attachment === null) {
            return self::DELETE;
        }
        if (($attachment['disk'] ?? null) !== 'g7mediabooster'
            || ($attachment['collection'] ?? null) !== 'post_attachments'
            || ! is_string($attachment['path'] ?? null)
            || strtolower($attachment['path']) !== strtolower($uploadId)
        ) {
            return self::BLOCK;
        }

        return ($attachment['deleted_at'] ?? null) === null ? self::CANCEL : self::DELETE;
    }
}
