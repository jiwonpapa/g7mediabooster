<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use Throwable;
use UnexpectedValueException;

final class Hooks
{
    /** @var list<array<string, mixed>> */
    private static array $pending = [];

    /** @param array<string, mixed> $board */
    public static function writeForm(array $board, int|string $writeId, string $mode): void
    {
        try {
            $runtime = new GnuboardRuntime;
            $configuration = $runtime->configuration();
            if (! $configuration->enabled) {
                return;
            }
            $runtime->assertUploadPermission($board);
            $payload = json_encode([
                'apiUrl' => $runtime->apiUrl(),
                'assetUrl' => G5_PLUGIN_URL.'/g7mediabooster/assets/uploader.iife.js',
                'boardTable' => (string) $board['bo_table'],
                'csrfToken' => $runtime->csrfToken(),
                'version' => '0.1.0',
            ], JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_HEX_TAG | JSON_HEX_AMP | JSON_HEX_APOS | JSON_HEX_QUOT);
            if (! is_string($payload)) {
                return;
            }
            add_javascript('<script>window.G7MediaBoosterG5Config='.trim($payload).';</script>', 90);
            add_javascript('<script src="'.G5_PLUGIN_URL.'/g7mediabooster/assets/uploader.iife.js" defer></script>', 91);
        } catch (Throwable $error) {
            error_log('G7MediaBooster G5 form hook disabled: '.get_class($error));
        }
    }

    /** @param array<string, mixed> $board */
    public static function beforeWrite(array $board, int|string $writeId, string $mode, string $query): void
    {
        self::$pending = [];
        $raw = $_POST['g7mb_upload_ids'] ?? null;
        if ($raw === null || $raw === '') {
            return;
        }
        try {
            if (! is_string($raw) || strlen($raw) > 4096) {
                throw new UnexpectedValueException('invalid upload list');
            }
            // G5 escapes every POST string before hooks run. A strict UUID CSV
            // avoids JSON quote escaping while remaining unambiguous.
            $uploadIds = explode(',', $raw);
            if (count($uploadIds) < 1
                || count($uploadIds) > min(100, max(1, (int) ($board['bo_upload_count'] ?? 0)))
                || count(array_unique($uploadIds)) !== count($uploadIds)
            ) {
                throw new UnexpectedValueException('invalid upload list');
            }
            foreach ($uploadIds as $uploadId) {
                if (! is_string($uploadId)
                    || ! preg_match('/^[a-f0-9]{8}-[a-f0-9]{4}-[1-8][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/', $uploadId)
                ) {
                    throw new UnexpectedValueException('invalid upload id');
                }
            }
            $runtime = new GnuboardRuntime;
            if (! $runtime->configuration()->enabled) {
                throw new UnexpectedValueException('adapter disabled');
            }
            $runtime->assertUploadPermission($board);
            self::$pending = $runtime->store()->readyOwnedForLink(
                $uploadIds,
                $runtime->ownerKey(),
                (string) $board['bo_table'],
            );
            self::assertProjectedCount($board, (int) $writeId, $mode, count(self::$pending));
        } catch (Throwable) {
            self::$pending = [];
            alert('미디어 첨부를 확인할 수 없습니다. 업로드 완료 상태와 파일 개수를 다시 확인해 주십시오.');
        }
    }

    /** @param array<string, mixed> $board */
    public static function afterWrite(array $board, int|string $writeId, string $mode, string $query, string $redirectUrl): void
    {
        if (self::$pending === []) {
            return;
        }
        try {
            global $write_table;
            if (! is_string($write_table)) {
                throw new UnexpectedValueException('write table is unavailable');
            }
            $runtime = new GnuboardRuntime;
            $runtime->store()->linkReady(
                self::$pending,
                (string) $board['bo_table'],
                (int) $writeId,
                $write_table,
                min(100, max(1, (int) $board['bo_upload_count'])),
                $runtime->deliveryUrl(),
            );
            self::$pending = [];
        } catch (Throwable $error) {
            error_log('G7MediaBooster G5 attachment link failed: '.get_class($error));
            alert('게시글은 저장됐지만 미디어 첨부 연결에 실패했습니다. 관리자에게 복구를 요청해 주십시오.');
        }
    }

    /** @param array<mixed> $files @return array<mixed> */
    public static function filterFiles(array $files, string $boardTable, int|string $writeId): array
    {
        foreach ($files as $index => &$file) {
            if (! is_int($index)
                || ! is_array($file)
                || ($file['bf_storage'] ?? null) !== 'g7mediabooster'
                || ! is_string($file['bf_fileurl'] ?? null)
                || ! is_string($file['bf_thumburl'] ?? null)
            ) {
                continue;
            }
            $source = htmlspecialchars((string) ($file['source'] ?? 'media'), ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
            $href = htmlspecialchars((string) ($file['href'] ?? ''), ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
            $master = htmlspecialchars($file['bf_fileurl'], ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
            $thumbnail = htmlspecialchars($file['bf_thumburl'], ENT_QUOTES | ENT_SUBSTITUTE, 'UTF-8');
            $extension = strtolower(pathinfo((string) ($file['file'] ?? ''), PATHINFO_EXTENSION));
            if (in_array($extension, ['mp4', 'mov'], true)) {
                $videoType = $extension === 'mov' ? 'video/quicktime' : 'video/mp4';
                $file['view'] = '<video controls preload="metadata" poster="'.$thumbnail.'" style="max-width:100%;height:auto">'.
                    '<source src="'.$master.'" type="'.$videoType.'"></video>';
            } else {
                $file['view'] = '<a href="'.$href.'"><img src="'.$thumbnail.'" alt="'.$source.'" loading="lazy" decoding="async" style="max-width:100%;height:auto"></a>';
            }
        }
        unset($file);

        return $files;
    }

    /** @param array<string, mixed> $file */
    public static function remoteFileExists(bool $exists, array $file): bool
    {
        return ($file['bf_storage'] ?? null) === 'g7mediabooster' ? true : $exists;
    }

    /** @param array<string, mixed> $file */
    public static function downloadHeader(array $file, bool $fileExists): void
    {
        if (($file['bf_storage'] ?? null) !== 'g7mediabooster') {
            return;
        }
        global $bo_table, $wr_id;
        try {
            (new RemoteDelivery(new GnuboardRuntime))->redirect($file, (string) $bo_table, (int) $wr_id, 'master');
        } catch (Throwable $error) {
            error_log('G7MediaBooster G5 download denied: '.get_class($error));
            http_response_code(404);
            exit;
        }
    }

    /** @param array<string, mixed> $file */
    public static function scheduleRemoteDeletion(string $path, array $file): string
    {
        if (($file['bf_storage'] ?? null) !== 'g7mediabooster') {
            return $path;
        }
        try {
            $runtime = new GnuboardRuntime;
            $runtime->store()->scheduleDeletionForFile($file, $runtime->configuration()->deleteRetentionDays);
        } catch (Throwable $error) {
            error_log('G7MediaBooster G5 deletion schedule failed: '.get_class($error));
        }

        return $path;
    }

    /** @param array<string, mixed> $board */
    private static function assertProjectedCount(array $board, int $writeId, string $mode, int $remoteCount): void
    {
        global $g5;
        $existing = 0;
        if ($mode === 'u' && $writeId > 0) {
            $row = sql_fetch("SELECT COUNT(*) AS `cnt` FROM `{$g5['board_file_table']}` WHERE `bo_table` = '".
                sql_real_escape_string((string) $board['bo_table'])."' AND `wr_id` = ".$writeId, false);
            $existing = is_array($row) ? (int) ($row['cnt'] ?? 0) : 0;
            foreach ((array) ($_POST['bf_file_del'] ?? []) as $delete) {
                if ($delete) {
                    $existing = max(0, $existing - 1);
                }
            }
        }
        $local = 0;
        foreach ((array) ($_FILES['bf_file']['name'] ?? []) as $name) {
            if (is_string($name) && trim($name) !== '') {
                $local++;
            }
        }
        if ($existing + $local + $remoteCount > min(100, max(1, (int) $board['bo_upload_count']))) {
            throw new UnexpectedValueException('projected attachment count exceeds board limit');
        }
    }
}
