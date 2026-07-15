<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

final class AttachmentUrlResolver
{
    public function resolve(
        ?string $defaultUrl,
        mixed $attachment,
        string $variant,
        ?string $boardSlug = null,
    ): ?string {
        if (! in_array($variant, ['master', 'thumbnail'], true) || ! is_object($attachment)) {
            return $defaultUrl;
        }

        $disk = $attachment->disk ?? null;
        $uploadId = $attachment->path ?? null;
        $hash = $attachment->hash ?? null;
        if ($disk !== 'g7mediabooster'
            || ! is_string($uploadId)
            || ! preg_match('/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/', $uploadId)
            || ! is_string($hash)
            || ! preg_match('/^[A-Za-z0-9]{12}$/', $hash)
        ) {
            return $defaultUrl;
        }

        if ($boardSlug === null && is_string($defaultUrl)) {
            preg_match(
                '#^/api/modules/sirsoft-board/boards/([A-Za-z0-9_-]+)/attachment/[A-Za-z0-9]{12}(?:/preview)?$#',
                $defaultUrl,
                $matches,
            );
            $boardSlug = $matches[1] ?? null;
        }
        if (! is_string($boardSlug) || ! preg_match('/^[A-Za-z0-9_-]+$/', $boardSlug)) {
            return $defaultUrl;
        }

        return sprintf(
            '/api/modules/jiwonpapa-g7mediabooster/boards/%s/attachments/%s/%s',
            $boardSlug,
            $hash,
            $variant,
        );
    }
}
