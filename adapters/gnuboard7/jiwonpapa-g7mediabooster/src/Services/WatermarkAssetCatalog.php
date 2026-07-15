<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use Illuminate\Support\Facades\DB;
use JsonException;
use stdClass;

final class WatermarkAssetCatalog
{
    private const MAX_SOURCE_BYTES = 16 * 1024 * 1024;
    private const MAX_THUMBNAIL_BYTES = 32 * 1024 * 1024;
    private const MAX_ASSETS = 50;
    private const SOURCE_TYPES = ['image/jpeg', 'image/png', 'image/webp'];

    /**
     * Only returns recent Ready image uploads owned by the current admin.
     * Native attachment metadata is revalidated so a hand-written DB row
     * cannot become a watermark source through this catalog.
     *
     * @return list<array<string, int|string>>
     */
    public function forUser(int $userId): array
    {
        if ($userId < 1) {
            return [];
        }

        return DB::table('g7mb_upload_sessions as sessions')
            ->join('board_attachments as attachments', 'attachments.id', '=', 'sessions.attachment_id')
            ->where('sessions.user_id', $userId)
            ->where('sessions.state', 'ready')
            ->where('sessions.declared_kind', 'image')
            ->whereBetween('sessions.expected_size_bytes', [1, self::MAX_SOURCE_BYTES])
            ->whereNotNull('sessions.materialized_at')
            ->where('sessions.ownership_expires_at', '>', now())
            ->whereNull('attachments.deleted_at')
            ->where('attachments.disk', 'g7mediabooster')
            ->where('attachments.collection', 'post_attachments')
            ->where('attachments.created_by', $userId)
            ->where('attachments.mime_type', 'image/jpeg')
            ->orderByDesc('sessions.created_at')
            ->limit(self::MAX_ASSETS)
            ->get([
                'sessions.upload_id',
                'sessions.board_slug',
                'sessions.original_filename',
                'sessions.expected_size_bytes',
                'sessions.created_at',
                'attachments.path',
                'attachments.stored_filename',
                'attachments.meta',
            ])
            ->map(fn (stdClass $row): ?array => $this->validatedAsset($row))
            ->filter(static fn (?array $asset): bool => $asset !== null)
            ->values()
            ->all();
    }

    public function isSelectableForUser(int $userId, string $uploadId): bool
    {
        $uploadId = strtolower(trim($uploadId));
        if (! $this->isUuid($uploadId)) {
            return false;
        }

        foreach ($this->forUser($userId) as $asset) {
            if ($asset['upload_id'] === $uploadId) {
                return true;
            }
        }

        return false;
    }

    /** @return array<string, int|string>|null */
    private function validatedAsset(stdClass $row): ?array
    {
        $uploadId = is_string($row->upload_id ?? null) ? strtolower($row->upload_id) : '';
        $boardSlug = $row->board_slug ?? null;
        $filename = $row->original_filename ?? null;
        $sourceBytes = filter_var($row->expected_size_bytes ?? null, FILTER_VALIDATE_INT);
        if (! $this->isUuid($uploadId)
            || ! is_string($boardSlug)
            || preg_match('/^[A-Za-z0-9_-]{1,100}$/', $boardSlug) !== 1
            || ! is_string($filename)
            || $filename === ''
            || mb_strlen($filename, 'UTF-8') > 255
            || preg_match('#[\x00-\x1F\x7F/\\\\]#u', $filename) === 1
            || ! is_int($sourceBytes)
            || $sourceBytes < 1
            || $sourceBytes > self::MAX_SOURCE_BYTES
            || ($row->path ?? null) !== $uploadId
            || ($row->stored_filename ?? null) !== $uploadId.'.jpg'
        ) {
            return null;
        }

        try {
            $meta = is_string($row->meta ?? null)
                ? json_decode($row->meta, true, 32, JSON_THROW_ON_ERROR)
                : null;
        } catch (JsonException) {
            return null;
        }
        $detectedType = is_array($meta) ? ($meta['g7mb_detected_content_type'] ?? null) : null;
        $thumbnailSize = is_array($meta)
            ? filter_var($meta['g7mb_thumbnail_size'] ?? null, FILTER_VALIDATE_INT)
            : false;
        if (! is_array($meta)
            || ($meta['g7mb_upload_id'] ?? null) !== $uploadId
            || ! is_string($detectedType)
            || ! in_array($detectedType, self::SOURCE_TYPES, true)
            || ($meta['g7mb_thumbnail_content_type'] ?? null) !== 'image/jpeg'
            || ! is_int($thumbnailSize)
            || $thumbnailSize < 1
            || $thumbnailSize > self::MAX_THUMBNAIL_BYTES
        ) {
            return null;
        }

        return [
            'upload_id' => $uploadId,
            'filename' => $filename,
            'source_bytes' => $sourceBytes,
            'detected_content_type' => $detectedType,
            'board_slug' => $boardSlug,
            'created_at' => (string) ($row->created_at ?? ''),
        ];
    }

    private function isUuid(string $value): bool
    {
        return preg_match(
            '/^[a-f0-9]{8}-[a-f0-9]{4}-[1-8][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/',
            $value,
        ) === 1;
    }
}
