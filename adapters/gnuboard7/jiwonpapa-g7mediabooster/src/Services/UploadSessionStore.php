<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use Illuminate\Support\Facades\DB;
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
        foreach ($requestedFiles as $file) {
            $clientRef = $file['client_ref'] ?? null;
            if (! is_string($clientRef) || isset($filesByRef[$clientRef])) {
                throw new UnexpectedValueException('invalid upload request mapping');
            }
            $filesByRef[$clientRef] = $file;
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

            $file = $filesByRef[$clientRef];
            $rows[] = [
                'upload_id' => strtolower($uploadId),
                'batch_id' => strtolower($batchId),
                'user_id' => $userId,
                'board_slug' => $boardSlug,
                'client_ref' => $clientRef,
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
