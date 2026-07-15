<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use UnexpectedValueException;

final class AttachmentMaterializationValidator
{
    private const IMAGE_MAX_BYTES = 128 * 1024 * 1024;
    private const VIDEO_MAX_BYTES = 5 * 1024 * 1024 * 1024;
    private const THUMBNAIL_MAX_BYTES = 32 * 1024 * 1024;

    /**
     * @param array<string, mixed> $status
     * @param array<string, mixed> $session
     * @return array<string, mixed>
     */
    public function validate(array $status, array $session): array
    {
        $uploadId = $this->requiredUuid($session['upload_id'] ?? null);
        if ($this->requiredUuid($status['upload_id'] ?? null) !== $uploadId
            || ($status['state'] ?? null) !== 'ready'
            || ($status['deletion_pending'] ?? null) !== false
        ) {
            throw new UnexpectedValueException('upload is not a deliverable Ready asset');
        }

        $kind = $session['declared_kind'] ?? null;
        $expectedSize = filter_var($session['expected_size_bytes'] ?? null, FILTER_VALIDATE_INT);
        $attachmentOrder = filter_var($session['attachment_order'] ?? null, FILTER_VALIDATE_INT);
        if (! in_array($kind, ['image', 'video'], true)
            || ! is_int($expectedSize)
            || $expectedSize < 1
            || ! is_int($attachmentOrder)
            || $attachmentOrder < 1
            || $attachmentOrder > 100
            || ($kind === 'image' && $expectedSize > self::IMAGE_MAX_BYTES)
            || ($kind === 'video' && $expectedSize > self::VIDEO_MAX_BYTES)
        ) {
            throw new UnexpectedValueException('stored upload reservation is invalid');
        }

        $detectedType = $status['detected_content_type'] ?? null;
        $allowedDetectedTypes = $kind === 'image'
            ? ['image/avif', 'image/gif', 'image/heic', 'image/heif', 'image/jpeg', 'image/png', 'image/webp']
            : ['video/mp4', 'video/quicktime'];
        if (! is_string($detectedType) || ! in_array($detectedType, $allowedDetectedTypes, true)) {
            throw new UnexpectedValueException('detected media type is not release-supported');
        }

        $derivatives = $status['derivatives'] ?? null;
        if (! is_array($derivatives) || ! array_is_list($derivatives) || count($derivatives) !== 2) {
            throw new UnexpectedValueException('Ready asset must have exactly two derivatives');
        }

        $byVariant = [];
        foreach ($derivatives as $derivative) {
            if (! is_array($derivative)) {
                throw new UnexpectedValueException('invalid derivative');
            }
            $variant = $derivative['variant'] ?? null;
            if (! is_string($variant) || ! in_array($variant, ['master', 'thumbnail'], true) || isset($byVariant[$variant])) {
                throw new UnexpectedValueException('invalid derivative set');
            }
            $this->validateDerivative($derivative, $variant, $kind, $detectedType, $expectedSize);
            $byVariant[$variant] = $derivative;
        }
        if (! isset($byVariant['master'], $byVariant['thumbnail'])) {
            throw new UnexpectedValueException('incomplete derivative set');
        }

        $master = $byVariant['master'];
        $thumbnail = $byVariant['thumbnail'];
        if ($master['preset_id'] !== $thumbnail['preset_id']) {
            throw new UnexpectedValueException('derivative preset mismatch');
        }

        $extension = match ($master['content_type']) {
            'image/jpeg' => 'jpg',
            'video/mp4' => 'mp4',
            'video/quicktime' => 'mov',
            default => throw new UnexpectedValueException('invalid master derivative type'),
        };
        $originalFilename = $this->normalizedFilename($session['original_filename'] ?? null, $extension);

        return [
            'board_id' => 0,
            'post_id' => null,
            'temp_key' => null,
            'original_filename' => $originalFilename,
            'stored_filename' => $uploadId.'.'.$extension,
            'disk' => 'g7mediabooster',
            'path' => $uploadId,
            'mime_type' => $master['content_type'],
            'size' => $master['byte_len'],
            'collection' => 'post_attachments',
            'order' => $attachmentOrder,
            'meta' => [
                'g7mb_upload_id' => $uploadId,
                'g7mb_preset_id' => $master['preset_id'],
                'g7mb_detected_content_type' => $detectedType,
                'g7mb_thumbnail_content_type' => $thumbnail['content_type'],
                'g7mb_thumbnail_size' => $thumbnail['byte_len'],
            ],
        ];
    }

    /** @param array<string, mixed> $derivative */
    private function validateDerivative(
        array $derivative,
        string $variant,
        string $kind,
        string $detectedType,
        int $expectedSize,
    ): void
    {
        $presetId = $derivative['preset_id'] ?? null;
        $urlPath = $derivative['url_path'] ?? null;
        $contentType = $derivative['content_type'] ?? null;
        $byteLen = $derivative['byte_len'] ?? null;
        if (! is_string($presetId)
            || ! preg_match('/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/', $presetId)
            || ! is_string($urlPath)
            || strlen($urlPath) < 1
            || strlen($urlPath) > 1024
            || str_contains($urlPath, "\0")
            || ! is_int($byteLen)
            || $byteLen < 1
        ) {
            throw new UnexpectedValueException('invalid derivative metadata');
        }

        if ($variant === 'thumbnail') {
            if ($contentType !== 'image/jpeg' || $byteLen > self::THUMBNAIL_MAX_BYTES) {
                throw new UnexpectedValueException('invalid thumbnail derivative');
            }

            return;
        }

        $expectedContentType = $kind === 'image' ? 'image/jpeg' : $detectedType;
        $maxBytes = $kind === 'image' ? self::IMAGE_MAX_BYTES : self::VIDEO_MAX_BYTES;
        if ($contentType !== $expectedContentType || $byteLen > $maxBytes) {
            throw new UnexpectedValueException('invalid master derivative');
        }
        if ($kind === 'video' && $byteLen !== $expectedSize) {
            throw new UnexpectedValueException('video master length mismatch');
        }
    }

    private function requiredUuid(mixed $value): string
    {
        if (! is_string($value) || ! preg_match(
            '/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/',
            $value,
        )) {
            throw new UnexpectedValueException('invalid upload id');
        }

        return strtolower($value);
    }

    private function normalizedFilename(mixed $value, string $extension): string
    {
        if (! is_string($value)
            || $value === ''
            || mb_strlen($value, 'UTF-8') > 255
            || preg_match('#[\x00-\x1F\x7F/\\\\]#u', $value)
        ) {
            throw new UnexpectedValueException('invalid original filename');
        }

        $stem = pathinfo($value, PATHINFO_FILENAME);
        $stem = trim($stem, " .\t\n\r\0\x0B");
        if ($stem === '') {
            $stem = 'media';
        }
        $suffix = '.'.$extension;
        while (mb_strlen($stem.$suffix, 'UTF-8') > 255) {
            $stem = mb_substr($stem, 0, max(1, mb_strlen($stem, 'UTF-8') - 1), 'UTF-8');
        }

        return $stem.$suffix;
    }
}
