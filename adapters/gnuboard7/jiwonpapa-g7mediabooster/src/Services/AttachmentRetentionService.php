<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use Illuminate\Support\Facades\DB;
use RuntimeException;
use stdClass;

final class AttachmentRetentionService
{
    private const SESSION_TABLE = 'g7mb_upload_sessions';
    private const ATTACHMENT_TABLE = 'board_attachments';
    private const MAX_ATTEMPTS = 10;

    public function __construct(private readonly AttachmentRetentionDecision $decision) {}

    public function schedulePostDeletion(int $postId, string $boardSlug, int $retentionDays): int
    {
        if ($postId < 1 || ! $this->validSlug($boardSlug) || $retentionDays < 1 || $retentionDays > 365) {
            return 0;
        }
        $attachmentIds = DB::table(self::ATTACHMENT_TABLE)
            ->where('post_id', $postId)
            ->where('disk', 'g7mediabooster')
            ->where('collection', 'post_attachments')
            ->whereNotNull('deleted_at')
            ->pluck('id')
            ->all();

        return $this->scheduleIds($attachmentIds, $boardSlug, 'post_delete', $retentionDays);
    }

    public function scheduleAttachmentDeletion(int $attachmentId, string $boardSlug, int $retentionDays): int
    {
        if ($attachmentId < 1 || ! $this->validSlug($boardSlug) || $retentionDays < 1 || $retentionDays > 365) {
            return 0;
        }
        $eligible = DB::table(self::ATTACHMENT_TABLE)
            ->where('id', $attachmentId)
            ->where('disk', 'g7mediabooster')
            ->where('collection', 'post_attachments')
            ->whereNotNull('deleted_at')
            ->exists();

        return $eligible ? $this->scheduleIds([$attachmentId], $boardSlug, 'attachment_delete', $retentionDays) : 0;
    }

    public function cancelRestoredPost(int $postId, string $boardSlug): int
    {
        if ($postId < 1 || ! $this->validSlug($boardSlug)) {
            return 0;
        }
        $attachmentIds = DB::table(self::ATTACHMENT_TABLE)
            ->where('post_id', $postId)
            ->where('disk', 'g7mediabooster')
            ->where('collection', 'post_attachments')
            ->where('trigger_type', 'cascade')
            ->whereNull('deleted_at')
            ->pluck('id')
            ->all();
        if ($attachmentIds === []) {
            return 0;
        }

        return DB::table(self::SESSION_TABLE)
            ->where('board_slug', $boardSlug)
            ->whereIn('attachment_id', $attachmentIds)
            ->whereNull('retention_request_started_at')
            ->whereNull('deletion_requested_at')
            ->update($this->clearedRetention());
    }

    public function preparePostRestore(int $postId, string $boardSlug): void
    {
        if ($postId < 1 || ! $this->validSlug($boardSlug)) {
            return;
        }
        $attachmentIds = DB::table(self::ATTACHMENT_TABLE)
            ->where('post_id', $postId)
            ->where('disk', 'g7mediabooster')
            ->where('collection', 'post_attachments')
            ->where('trigger_type', 'cascade')
            ->pluck('id')
            ->all();
        if ($attachmentIds === []) {
            return;
        }
        DB::transaction(function () use ($attachmentIds, $boardSlug): void {
            $sessions = DB::table(self::SESSION_TABLE)
                ->where('board_slug', $boardSlug)
                ->whereIn('attachment_id', $attachmentIds)
                ->lockForUpdate()
                ->get();
            foreach ($sessions as $session) {
                if ($session instanceof stdClass
                    && ($session->retention_request_started_at !== null || $session->deletion_requested_at !== null)
                ) {
                    throw new RuntimeException('G7_MEDIA_RETENTION_ALREADY_STARTED');
                }
            }

            DB::table(self::SESSION_TABLE)
                ->where('board_slug', $boardSlug)
                ->whereIn('attachment_id', $attachmentIds)
                ->whereNull('retention_request_started_at')
                ->whereNull('deletion_requested_at')
                ->update($this->clearedRetention());
        });
    }

    /** @return list<array<string, mixed>> */
    public function claimDue(int $limit): array
    {
        $limit = max(1, min(100, $limit));

        return DB::transaction(function () use ($limit): array {
            $now = now();
            $rows = DB::table(self::SESSION_TABLE)
                ->whereNotNull('attachment_id')
                ->whereNotNull('retention_delete_after')
                ->where('retention_delete_after', '<=', $now)
                ->whereNull('deletion_requested_at')
                ->where('retention_attempts', '<', self::MAX_ATTEMPTS)
                ->where(function ($query) use ($now): void {
                    $query->where(function ($fresh): void {
                        $fresh->whereNull('retention_request_started_at')->whereNull('retention_lease_until');
                    })->orWhere('retention_lease_until', '<=', $now);
                })
                ->orderBy('retention_delete_after')
                ->limit($limit)
                ->lockForUpdate()
                ->get();

            $claimed = [];
            foreach ($rows as $row) {
                if (! $row instanceof stdClass) {
                    continue;
                }
                $updated = DB::table(self::SESSION_TABLE)
                    ->where('upload_id', $row->upload_id)
                    ->whereNull('deletion_requested_at')
                    ->update([
                        'retention_attempts' => DB::raw('retention_attempts + 1'),
                        'retention_lease_until' => $now->copy()->addMinutes(10),
                        'retention_last_error' => null,
                        'updated_at' => $now,
                    ]);
                if ($updated === 1) {
                    $claimed[] = (array) $row;
                }
            }

            return $claimed;
        });
    }

    public function beginClaim(array $session): string
    {
        $attachmentId = filter_var($session['attachment_id'] ?? null, FILTER_VALIDATE_INT);
        $uploadId = $session['upload_id'] ?? null;
        if (! is_int($attachmentId) || $attachmentId < 1 || ! is_string($uploadId)) {
            return AttachmentRetentionDecision::BLOCK;
        }

        return DB::transaction(function () use ($attachmentId, $uploadId): string {
            $stored = DB::table(self::SESSION_TABLE)
                ->where('upload_id', strtolower($uploadId))
                ->lockForUpdate()
                ->first();
            if (! $stored instanceof stdClass
                || $stored->retention_delete_after === null
                || $stored->deletion_requested_at !== null
                || $stored->retention_lease_until === null
            ) {
                return AttachmentRetentionDecision::CANCEL;
            }
            $row = DB::table(self::ATTACHMENT_TABLE)->where('id', $attachmentId)->lockForUpdate()->first();
            $decision = $this->decision->evaluate($row instanceof stdClass ? (array) $row : null, $uploadId);
            if ($decision === AttachmentRetentionDecision::CANCEL) {
                DB::table(self::SESSION_TABLE)
                    ->where('upload_id', strtolower($uploadId))
                    ->update($this->clearedRetention());
            } elseif ($decision === AttachmentRetentionDecision::BLOCK) {
                DB::table(self::SESSION_TABLE)
                    ->where('upload_id', strtolower($uploadId))
                    ->update([
                        'retention_attempts' => self::MAX_ATTEMPTS,
                        'retention_lease_until' => null,
                        'retention_last_error' => 'ATTACHMENT_MAPPING_MISMATCH',
                        'updated_at' => now(),
                    ]);
            } elseif ($stored->retention_request_started_at === null) {
                DB::table(self::SESSION_TABLE)
                    ->where('upload_id', strtolower($uploadId))
                    ->update([
                        'state' => 'deletion_requesting',
                        'retention_request_started_at' => now(),
                        'updated_at' => now(),
                    ]);
            }

            return $decision;
        });
    }

    public function cancelClaim(string $uploadId): void
    {
        DB::table(self::SESSION_TABLE)->where('upload_id', strtolower($uploadId))->update($this->clearedRetention());
    }

    public function completeClaim(string $uploadId): void
    {
        DB::table(self::SESSION_TABLE)
            ->where('upload_id', strtolower($uploadId))
            ->update([
                'state' => 'deletion_pending',
                'deletion_requested_at' => now(),
                'retention_delete_after' => null,
                'retention_lease_until' => null,
                'retention_request_started_at' => null,
                'retention_last_error' => null,
                'updated_at' => now(),
            ]);
    }

    public function failClaim(
        string $uploadId,
        string $errorCode,
        bool $permanent = false,
        bool $keepInFlight = false,
    ): void
    {
        $safeCode = preg_match('/^[A-Z0-9_]{1,80}$/', $errorCode) === 1 ? $errorCode : 'RETENTION_REQUEST_FAILED';
        $row = DB::table(self::SESSION_TABLE)->where('upload_id', strtolower($uploadId))->first();
        $attempts = $row instanceof stdClass ? max(1, (int) ($row->retention_attempts ?? 1)) : 1;
        $requestStartedAt = $row instanceof stdClass ? ($row->retention_request_started_at ?? now()) : now();
        $backoffMinutes = min(24 * 60, 15 * (2 ** min(6, $attempts - 1)));
        DB::table(self::SESSION_TABLE)
            ->where('upload_id', strtolower($uploadId))
            ->update([
                'retention_attempts' => $permanent ? self::MAX_ATTEMPTS : $attempts,
                'retention_lease_until' => $permanent ? null : now()->addMinutes($backoffMinutes),
                'retention_request_started_at' => $keepInFlight ? $requestStartedAt : null,
                'retention_last_error' => $safeCode,
                'state' => $keepInFlight ? 'deletion_requesting' : 'ready',
                'updated_at' => now(),
            ]);
    }

    /** @param array<int, mixed> $attachmentIds */
    private function scheduleIds(array $attachmentIds, string $boardSlug, string $reason, int $retentionDays): int
    {
        $ids = [];
        foreach ($attachmentIds as $attachmentId) {
            $normalized = filter_var($attachmentId, FILTER_VALIDATE_INT);
            if (is_int($normalized) && $normalized > 0) {
                $ids[] = $normalized;
            }
        }
        $ids = array_values(array_unique($ids));
        if ($ids === []) {
            return 0;
        }

        return DB::table(self::SESSION_TABLE)
            ->where('board_slug', $boardSlug)
            ->whereIn('attachment_id', $ids)
            ->whereNull('retention_request_started_at')
            ->whereNull('deletion_requested_at')
            ->update([
                'retention_delete_after' => now()->addDays($retentionDays),
                'retention_reason' => $reason,
                'retention_attempts' => 0,
                'retention_lease_until' => null,
                'retention_last_error' => null,
                'updated_at' => now(),
            ]);
    }

    /** @return array<string, mixed> */
    private function clearedRetention(): array
    {
        return [
            'retention_delete_after' => null,
            'retention_reason' => null,
            'retention_attempts' => 0,
            'retention_lease_until' => null,
            'retention_request_started_at' => null,
            'retention_last_error' => null,
            'updated_at' => now(),
        ];
    }

    private function validSlug(string $slug): bool
    {
        return preg_match('/^[A-Za-z0-9_-]+$/', $slug) === 1;
    }
}
