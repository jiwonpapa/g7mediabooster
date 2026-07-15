<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use Illuminate\Support\Facades\DB;
use stdClass;
use UnexpectedValueException;

final class UploadSessionStore
{
    private const TABLE = 'g7mb_upload_sessions';

    /**
     * @param array<int, array<string, mixed>> $requestedFiles
     * @param array<string, mixed> $createdBatch
     */
    public function recordBatch(
        int $userId,
        string $boardSlug,
        array $requestedFiles,
        array $createdBatch,
    ): void {
        $batchId = $createdBatch['batch_id'] ?? null;
        $uploads = $createdBatch['uploads'] ?? null;
        if (! is_string($batchId) || ! $this->isUuid($batchId) || ! is_array($uploads)) {
            throw new UnexpectedValueException('invalid upload batch response');
        }

        $filesByRef = [];
        $order = 0;
        foreach ($requestedFiles as $file) {
            $order++;
            $clientRef = $file['client_ref'] ?? null;
            if (! is_string($clientRef) || isset($filesByRef[$clientRef])) {
                throw new UnexpectedValueException('invalid upload request mapping');
            }
            $filesByRef[$clientRef] = [
                'file' => $file,
                'order' => $order,
            ];
        }
        if (count($filesByRef) !== count($uploads)) {
            throw new UnexpectedValueException('upload response count mismatch');
        }

        $now = now();
        $ownershipExpiresAt = $now->copy()->addDays(7);
        $rows = [];
        foreach ($uploads as $upload) {
            if (! is_array($upload)) {
                throw new UnexpectedValueException('invalid upload response item');
            }
            $clientRef = $upload['client_ref'] ?? null;
            $uploadId = $upload['upload_id'] ?? null;
            $method = $upload['method'] ?? null;
            if (
                ! is_string($clientRef)
                || ! isset($filesByRef[$clientRef])
                || ! is_string($uploadId)
                || ! $this->isUuid($uploadId)
                || ! in_array($method, ['single_put', 'multipart'], true)
            ) {
                throw new UnexpectedValueException('invalid upload response item');
            }

            $requested = $filesByRef[$clientRef];
            $file = $requested['file'];
            $rows[] = [
                'upload_id' => strtolower($uploadId),
                'batch_id' => strtolower($batchId),
                'user_id' => $userId,
                'board_slug' => $boardSlug,
                'client_ref' => $clientRef,
                'original_filename' => (string) ($file['original_filename'] ?? ''),
                'declared_kind' => (string) ($file['declared_kind'] ?? ''),
                'content_type_hint' => (string) ($file['content_type_hint'] ?? ''),
                'attachment_order' => (int) $requested['order'],
                'transfer_method' => $method,
                'expected_size_bytes' => (int) ($file['content_length'] ?? 0),
                'state' => 'created',
                'ownership_expires_at' => $ownershipExpiresAt,
                'created_at' => $now,
                'updated_at' => $now,
            ];
            unset($filesByRef[$clientRef]);
        }
        if ($filesByRef !== []) {
            throw new UnexpectedValueException('upload response mapping mismatch');
        }

        DB::transaction(static function () use ($rows): void {
            DB::table(self::TABLE)->insert($rows);
        });
    }

    public function isOwnedBy(string $uploadId, int $userId, string $boardSlug): bool
    {
        return DB::table(self::TABLE)
            ->where('upload_id', strtolower($uploadId))
            ->where('user_id', $userId)
            ->where('board_slug', $boardSlug)
            ->where('ownership_expires_at', '>', now())
            ->exists();
    }

    /**
     * @return array<string, mixed>|null
     */
    public function ownedSession(string $uploadId, int $userId, string $boardSlug): ?array
    {
        $row = DB::table(self::TABLE)
            ->where('upload_id', strtolower($uploadId))
            ->where('user_id', $userId)
            ->where('board_slug', $boardSlug)
            ->where('ownership_expires_at', '>', now())
            ->first();

        return $row instanceof stdClass ? (array) $row : null;
    }

    /**
     * The callback must only write the native attachment row. Network I/O is
     * deliberately completed before this transaction begins.
     *
     * @param callable():int $createAttachment
     */
    public function materializeAttachment(
        string $uploadId,
        int $userId,
        string $boardSlug,
        callable $createAttachment,
    ): int {
        return DB::transaction(function () use ($uploadId, $userId, $boardSlug, $createAttachment): int {
            $row = DB::table(self::TABLE)
                ->where('upload_id', strtolower($uploadId))
                ->where('user_id', $userId)
                ->where('board_slug', $boardSlug)
                ->where('ownership_expires_at', '>', now())
                ->lockForUpdate()
                ->first();
            if (! $row instanceof stdClass) {
                throw new UnexpectedValueException('upload session is not owned');
            }

            $existingId = filter_var($row->attachment_id ?? null, FILTER_VALIDATE_INT);
            if (is_int($existingId) && $existingId > 0) {
                return $existingId;
            }

            $attachmentId = $createAttachment();
            if ($attachmentId < 1) {
                throw new UnexpectedValueException('native attachment was not created');
            }

            $updated = DB::table(self::TABLE)
                ->where('upload_id', strtolower($uploadId))
                ->whereNull('attachment_id')
                ->update([
                    'attachment_id' => $attachmentId,
                    'materialized_at' => now(),
                    'state' => 'ready',
                    'updated_at' => now(),
                ]);
            if ($updated !== 1) {
                throw new UnexpectedValueException('attachment materialization lost its lock');
            }

            return $attachmentId;
        });
    }

    public function isMaterializedAs(string $uploadId, int $attachmentId, string $boardSlug): bool
    {
        return DB::table(self::TABLE)
            ->where('upload_id', strtolower($uploadId))
            ->where('attachment_id', $attachmentId)
            ->where('board_slug', $boardSlug)
            ->whereNotNull('materialized_at')
            ->exists();
    }

    public function markState(string $uploadId, string $state): void
    {
        DB::table(self::TABLE)
            ->where('upload_id', strtolower($uploadId))
            ->update(['state' => substr($state, 0, 32), 'updated_at' => now()]);
    }

    private function isUuid(string $value): bool
    {
        return (bool) preg_match(
            '/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/',
            $value,
        );
    }
}
