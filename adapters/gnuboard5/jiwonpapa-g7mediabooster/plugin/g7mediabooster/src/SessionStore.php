<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use RuntimeException;
use UnexpectedValueException;

final class SessionStore
{
    public function __construct(
        private readonly string $sessionTable,
        private readonly string $boardFileTable,
    ) {
        if (! preg_match('/^[A-Za-z0-9_]+$/', $sessionTable)
            || ! preg_match('/^[A-Za-z0-9_]+$/', $boardFileTable)
        ) {
            throw new RuntimeException('invalid Gnuboard table name');
        }
    }

    public function install(): void
    {
        $sql = "CREATE TABLE IF NOT EXISTS `{$this->sessionTable}` (
            `upload_id` char(36) NOT NULL,
            `batch_id` char(36) NOT NULL,
            `owner_key` varchar(80) NOT NULL,
            `bo_table` varchar(20) NOT NULL,
            `client_ref` varchar(64) NOT NULL,
            `original_filename` varchar(255) NOT NULL,
            `declared_kind` varchar(8) NOT NULL,
            `content_type_hint` varchar(255) NOT NULL,
            `expected_size_bytes` bigint unsigned NOT NULL,
            `attachment_order` smallint unsigned NOT NULL,
            `transfer_method` varchar(16) NOT NULL,
            `state` varchar(32) NOT NULL DEFAULT 'created',
            `ready_mime_type` varchar(64) DEFAULT NULL,
            `ready_master_bytes` bigint unsigned DEFAULT NULL,
            `ready_thumbnail_bytes` bigint unsigned DEFAULT NULL,
            `ready_preset_id` varchar(128) DEFAULT NULL,
            `wr_id` bigint unsigned DEFAULT NULL,
            `bf_no` smallint unsigned DEFAULT NULL,
            `deletion_due_at` datetime DEFAULT NULL,
            `deletion_attempts` smallint unsigned NOT NULL DEFAULT 0,
            `deletion_last_error` varchar(80) DEFAULT NULL,
            `deletion_requested_at` datetime DEFAULT NULL,
            `created_at` datetime NOT NULL,
            `updated_at` datetime NOT NULL,
            PRIMARY KEY (`upload_id`),
            UNIQUE KEY `g7mb_owner_client` (`owner_key`, `bo_table`, `client_ref`),
            KEY `g7mb_post` (`bo_table`, `wr_id`),
            KEY `g7mb_deletion_due` (`deletion_due_at`, `deletion_requested_at`)
        ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_unicode_ci";
        $this->execute($sql);
    }

    /**
     * @param list<array{client_ref:string,original_filename:string,declared_kind:string,content_length:int,content_type_hint:string}> $files
     * @param array{batch_id:string,uploads:list<array<string,mixed>>} $batch
     */
    public function recordBatch(string $ownerKey, string $boardTable, array $files, array $batch): void
    {
        $this->assertOwnerAndBoard($ownerKey, $boardTable);
        if (count($files) !== count($batch['uploads'])) {
            throw new UnexpectedValueException('batch persistence count mismatch');
        }
        $this->transaction(function () use ($ownerKey, $boardTable, $files, $batch): void {
            foreach ($files as $index => $file) {
                $upload = $batch['uploads'][$index];
                $now = $this->now();
                $sql = "INSERT INTO `{$this->sessionTable}`
                    (`upload_id`, `batch_id`, `owner_key`, `bo_table`, `client_ref`, `original_filename`,
                     `declared_kind`, `content_type_hint`, `expected_size_bytes`, `attachment_order`,
                     `transfer_method`, `state`, `created_at`, `updated_at`)
                    VALUES (".
                    $this->quote((string) $upload['upload_id']).','.
                    $this->quote($batch['batch_id']).','.
                    $this->quote($ownerKey).','.
                    $this->quote($boardTable).','.
                    $this->quote($file['client_ref']).','.
                    $this->quote($file['original_filename']).','.
                    $this->quote($file['declared_kind']).','.
                    $this->quote($file['content_type_hint']).','.
                    (string) $file['content_length'].','.
                    (string) ($index + 1).','.
                    $this->quote((string) $upload['method']).','.
                    $this->quote('created').','.
                    $this->quote($now).','.
                    $this->quote($now).
                    ')';
                $this->execute($sql);
            }
        });
    }

    /** @return array<string, mixed>|null */
    public function findOwned(string $uploadId, string $ownerKey, string $boardTable): ?array
    {
        $this->assertUploadId($uploadId);
        $this->assertOwnerAndBoard($ownerKey, $boardTable);
        $sql = "SELECT * FROM `{$this->sessionTable}` WHERE `upload_id` = ".
            $this->quote(strtolower($uploadId)).' AND `owner_key` = '.$this->quote($ownerKey).
            ' AND `bo_table` = '.$this->quote($boardTable).' LIMIT 1';

        return $this->fetch($sql);
    }

    /** @return array<string, mixed>|null */
    public function find(string $uploadId): ?array
    {
        $this->assertUploadId($uploadId);

        return $this->fetch("SELECT * FROM `{$this->sessionTable}` WHERE `upload_id` = ".
            $this->quote(strtolower($uploadId)).' LIMIT 1');
    }

    public function markState(string $uploadId, string $state): void
    {
        $this->assertUploadId($uploadId);
        if (! preg_match('/^[a-z_]{1,32}$/', $state)) {
            throw new UnexpectedValueException('invalid upload state');
        }
        $this->execute("UPDATE `{$this->sessionTable}` SET `state` = ".$this->quote($state).
            ', `updated_at` = '.$this->quote($this->now()).' WHERE `upload_id` = '.$this->quote(strtolower($uploadId)));
    }

    /** @param array<string, int|string> $asset */
    public function markReady(string $uploadId, array $asset): void
    {
        $this->assertUploadId($uploadId);
        $this->execute("UPDATE `{$this->sessionTable}` SET
            `state` = 'ready',
            `ready_mime_type` = ".$this->quote((string) $asset['mime_type']).",
            `ready_master_bytes` = ".(string) (int) $asset['size'].",
            `ready_thumbnail_bytes` = ".(string) (int) $asset['thumbnail_size'].",
            `ready_preset_id` = ".$this->quote((string) $asset['preset_id']).",
            `updated_at` = ".$this->quote($this->now())."
            WHERE `upload_id` = ".$this->quote(strtolower($uploadId)));
    }

    /** @param list<string> $uploadIds @return list<array<string, mixed>> */
    public function readyOwnedForLink(array $uploadIds, string $ownerKey, string $boardTable): array
    {
        if ($uploadIds === [] || count($uploadIds) > 100 || count(array_unique($uploadIds)) !== count($uploadIds)) {
            throw new UnexpectedValueException('invalid attachment upload list');
        }
        $this->assertOwnerAndBoard($ownerKey, $boardTable);
        $rows = [];
        foreach ($uploadIds as $uploadId) {
            $row = $this->findOwned($uploadId, $ownerKey, $boardTable);
            if ($row === null
                || ($row['state'] ?? null) !== 'ready'
                || ($row['wr_id'] ?? null) !== null
                || ! in_array($row['ready_mime_type'] ?? null, ['image/jpeg', 'video/mp4', 'video/quicktime'], true)
                || filter_var($row['ready_master_bytes'] ?? null, FILTER_VALIDATE_INT) === false
                || ! is_string($row['ready_preset_id'] ?? null)
            ) {
                throw new UnexpectedValueException('upload is not ready for this post');
            }
            $rows[] = $row;
        }

        return $rows;
    }

    /**
     * @param list<array<string, mixed>> $sessions
     * @return list<array{upload_id:string,bf_no:int}>
     */
    public function linkReady(
        array $sessions,
        string $boardTable,
        int $writeId,
        string $writeTable,
        int $maximumFiles,
        string $deliveryBaseUrl,
    ): array {
        if ($writeId < 1
            || $maximumFiles < 1
            || ! preg_match('/^[A-Za-z0-9_]+$/', $writeTable)
            || ! preg_match('/^https?:\/\//', $deliveryBaseUrl)
        ) {
            throw new UnexpectedValueException('invalid attachment link target');
        }

        $lockName = 'g7mb:'.substr(hash('sha256', $boardTable.':'.$writeId), 0, 48);

        return $this->withNamedLock($lockName, function () use (
            $sessions,
            $boardTable,
            $writeId,
            $writeTable,
            $maximumFiles,
            $deliveryBaseUrl,
        ): array {
            return $this->transaction(function () use (
                $sessions,
                $boardTable,
                $writeId,
                $writeTable,
                $maximumFiles,
                $deliveryBaseUrl,
            ): array {
                $post = $this->fetch("SELECT `wr_id` FROM `{$writeTable}` WHERE `wr_id` = ".(string) $writeId);
                if ($post === null) {
                    throw new UnexpectedValueException('post attachment target no longer exists');
                }
                $countRow = $this->fetch("SELECT COUNT(*) AS `cnt`, COALESCE(MAX(`bf_no`), -1) AS `max_no`
                    FROM `{$this->boardFileTable}` WHERE `bo_table` = ".$this->quote($boardTable).
                    ' AND `wr_id` = '.(string) $writeId);
                $existing = (int) ($countRow['cnt'] ?? 0);
                $next = (int) ($countRow['max_no'] ?? -1) + 1;
                $alreadyLinked = [];
                foreach ($sessions as $index => $session) {
                    $uploadId = (string) $session['upload_id'];
                    $existingFile = $this->fetch("SELECT `wr_id`, `bf_no` FROM `{$this->boardFileTable}`
                        WHERE `bo_table` = ".$this->quote($boardTable).
                        ' AND `bf_file` IN ('.$this->quote('g7mb-'.$uploadId.'.jpg').','.$this->quote('g7mb-'.$uploadId.'.mp4').') LIMIT 1');
                    if ($existingFile !== null) {
                        if ((int) $existingFile['wr_id'] !== $writeId) {
                            throw new UnexpectedValueException('upload is already linked to another post');
                        }
                        $alreadyLinked[$index] = (int) $existingFile['bf_no'];
                    }
                }
                if ($existing + count($sessions) - count($alreadyLinked) > $maximumFiles) {
                    throw new UnexpectedValueException('post attachment count exceeds the board limit');
                }

                $linked = [];
                foreach ($sessions as $index => $session) {
                    $uploadId = (string) $session['upload_id'];
                    $locked = $this->fetch("SELECT * FROM `{$this->sessionTable}` WHERE `upload_id` = ".
                        $this->quote($uploadId).' FOR UPDATE');
                    if ($locked === null || ($locked['state'] ?? null) !== 'ready') {
                        throw new UnexpectedValueException('upload link raced with another post');
                    }
                    if (isset($alreadyLinked[$index])) {
                        $existingNumber = $alreadyLinked[$index];
                        if (($locked['wr_id'] ?? null) !== null
                            && ((int) $locked['wr_id'] !== $writeId || (int) ($locked['bf_no'] ?? -1) !== $existingNumber)
                        ) {
                            throw new UnexpectedValueException('upload session link does not match the remote file row');
                        }
                        $this->execute("UPDATE `{$this->sessionTable}` SET `wr_id` = ".(string) $writeId.
                            ', `bf_no` = '.(string) $existingNumber.', `updated_at` = '.$this->quote($this->now()).
                            ' WHERE `upload_id` = '.$this->quote($uploadId));
                        $linked[] = ['upload_id' => $uploadId, 'bf_no' => $existingNumber];
                        continue;
                    }
                    if (($locked['wr_id'] ?? null) !== null) {
                        throw new UnexpectedValueException('upload link raced with another post');
                    }
                    $kind = (string) $locked['declared_kind'];
                    $extension = $kind === 'image' ? 'jpg' : 'mp4';
                    $filename = $this->normalizedFilename((string) $locked['original_filename'], $extension);
                    $stored = 'g7mb-'.$uploadId.'.'.$extension;
                    $size = (int) $locked['ready_master_bytes'];
                    $imageType = $kind === 'image' ? 2 : 0;
                    $base = $deliveryBaseUrl.'?bo_table='.rawurlencode($boardTable).
                        '&wr_id='.$writeId.'&no='.$next.'&variant=';
                    $sql = "INSERT INTO `{$this->boardFileTable}`
                        (`bo_table`, `wr_id`, `bf_no`, `bf_source`, `bf_file`, `bf_content`, `bf_fileurl`,
                         `bf_thumburl`, `bf_storage`, `bf_download`, `bf_filesize`, `bf_width`, `bf_height`,
                         `bf_type`, `bf_datetime`)
                        VALUES (".
                        $this->quote($boardTable).','.(string) $writeId.','.(string) $next.','.
                        $this->quote($filename).','.$this->quote($stored).','.$this->quote('').','.
                        $this->quote($base.'master').','.$this->quote($base.'thumbnail').','.
                        $this->quote('g7mediabooster').',0,'.(string) $size.',0,0,'.(string) $imageType.','.
                        $this->quote($this->now()).')';
                    $this->execute($sql);
                    $this->execute("UPDATE `{$this->sessionTable}` SET `wr_id` = ".(string) $writeId.
                        ', `bf_no` = '.(string) $next.', `updated_at` = '.$this->quote($this->now()).
                        ' WHERE `upload_id` = '.$this->quote($uploadId));
                    $linked[] = ['upload_id' => $uploadId, 'bf_no' => $next];
                    $next++;
                }

                $countRow = $this->fetch("SELECT COUNT(*) AS `cnt` FROM `{$this->boardFileTable}` WHERE `bo_table` = ".
                    $this->quote($boardTable).' AND `wr_id` = '.(string) $writeId);
                $count = (int) ($countRow['cnt'] ?? 0);
                $this->execute("UPDATE `{$writeTable}` SET `wr_file` = ".(string) $count.' WHERE `wr_id` = '.(string) $writeId);

                return $linked;
            });
        });
    }

    public function scheduleDeletionForFile(array $file, int $retentionDays): void
    {
        if (($file['bf_storage'] ?? null) !== 'g7mediabooster') {
            return;
        }
        $stored = (string) ($file['bf_file'] ?? '');
        if (! preg_match('/^g7mb-([a-f0-9-]{36})\.(?:jpg|mp4)$/i', $stored, $matches)) {
            return;
        }
        $due = gmdate('Y-m-d H:i:s', time() + max(0, min(365, $retentionDays)) * 86400);
        $this->execute("UPDATE `{$this->sessionTable}` SET `state` = 'deletion_scheduled',
            `deletion_due_at` = ".$this->quote($due).", `updated_at` = ".$this->quote($this->now())."
            WHERE `upload_id` = ".$this->quote(strtolower($matches[1]))." AND `deletion_requested_at` IS NULL");
    }

    /** @return list<array<string, mixed>> */
    public function dueDeletions(int $limit): array
    {
        $limit = max(1, min(100, $limit));
        $result = sql_query("SELECT * FROM `{$this->sessionTable}` WHERE `deletion_due_at` IS NOT NULL
            AND `deletion_due_at` <= UTC_TIMESTAMP() AND `deletion_requested_at` IS NULL
            AND `deletion_attempts` < 10 ORDER BY `deletion_due_at`, `upload_id` LIMIT {$limit}");
        if ($result === false) {
            throw new RuntimeException('G7MediaBooster deletion queue query failed');
        }
        $rows = [];
        while ($row = sql_fetch_array($result)) {
            $rows[] = $row;
        }

        return $rows;
    }

    public function completeDeletionRequest(string $uploadId): void
    {
        $this->assertUploadId($uploadId);
        $this->execute("UPDATE `{$this->sessionTable}` SET `state` = 'deletion_pending',
            `deletion_requested_at` = UTC_TIMESTAMP(), `deletion_last_error` = NULL,
            `updated_at` = UTC_TIMESTAMP() WHERE `upload_id` = ".$this->quote(strtolower($uploadId)));
    }

    public function failDeletionRequest(string $uploadId, string $errorCode): void
    {
        $this->assertUploadId($uploadId);
        $safeCode = preg_match('/^[A-Z0-9_]{1,80}$/', $errorCode) ? $errorCode : 'DELETE_REQUEST_FAILED';
        $this->execute("UPDATE `{$this->sessionTable}` SET `deletion_attempts` = `deletion_attempts` + 1,
            `deletion_last_error` = ".$this->quote($safeCode).",
            `deletion_due_at` = DATE_ADD(UTC_TIMESTAMP(), INTERVAL LEAST(3600, POW(2, `deletion_attempts` + 1) * 30) SECOND),
            `updated_at` = UTC_TIMESTAMP() WHERE `upload_id` = ".$this->quote(strtolower($uploadId)));
    }

    private function normalizedFilename(string $filename, string $extension): string
    {
        if ($filename === '' || mb_strlen($filename, 'UTF-8') > 255 || preg_match('#[\x00-\x1F\x7F/\\\\]#u', $filename)) {
            throw new UnexpectedValueException('stored original filename is invalid');
        }
        $stem = trim(pathinfo($filename, PATHINFO_FILENAME), " .\t\n\r\0\x0B");
        if ($stem === '') {
            $stem = 'media';
        }
        $suffix = '.'.$extension;
        while (mb_strlen($stem.$suffix, 'UTF-8') > 255) {
            $stem = mb_substr($stem, 0, max(1, mb_strlen($stem, 'UTF-8') - 1), 'UTF-8');
        }

        return $stem.$suffix;
    }

    private function assertUploadId(string $uploadId): void
    {
        if (! preg_match('/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/', $uploadId)) {
            throw new UnexpectedValueException('invalid upload id');
        }
    }

    private function assertOwnerAndBoard(string $ownerKey, string $boardTable): void
    {
        if (! preg_match('/^[mg]:[A-Za-z0-9_.:@-]{1,76}$/', $ownerKey)
            || ! preg_match('/^[A-Za-z0-9_]{1,20}$/', $boardTable)
        ) {
            throw new UnexpectedValueException('invalid upload ownership scope');
        }
    }

    /** @return array<string, mixed>|null */
    private function fetch(string $sql): ?array
    {
        $row = sql_fetch($sql, false);
        if ($row === false) {
            throw new RuntimeException('G7MediaBooster database query failed');
        }

        return is_array($row) && $row !== [] ? $row : null;
    }

    private function execute(string $sql): void
    {
        if (sql_query($sql, false) === false) {
            throw new RuntimeException('G7MediaBooster database mutation failed');
        }
    }

    private function quote(string $value): string
    {
        return "'".sql_real_escape_string($value)."'";
    }

    private function now(): string
    {
        return defined('G5_TIME_YMDHIS') ? G5_TIME_YMDHIS : gmdate('Y-m-d H:i:s');
    }

    /** @template T @param callable():T $callback @return T */
    private function withNamedLock(string $name, callable $callback): mixed
    {
        $lock = $this->fetch('SELECT GET_LOCK('.$this->quote($name).', 2) AS `acquired`');
        if ((int) ($lock['acquired'] ?? 0) !== 1) {
            throw new RuntimeException('G7MediaBooster post attachment lock is busy');
        }
        try {
            return $callback();
        } finally {
            sql_query('SELECT RELEASE_LOCK('.$this->quote($name).')', false);
        }
    }

    /** @template T @param callable():T $callback @return T */
    private function transaction(callable $callback): mixed
    {
        $this->execute('START TRANSACTION');
        try {
            $result = $callback();
            $this->execute('COMMIT');

            return $result;
        } catch (\Throwable $error) {
            sql_query('ROLLBACK', false);
            throw $error;
        }
    }
}
