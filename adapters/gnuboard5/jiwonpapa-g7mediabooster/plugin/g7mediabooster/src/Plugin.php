<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

final class Plugin
{
    private static bool $registered = false;

    public static function register(): void
    {
        if (self::$registered) {
            return;
        }
        self::$registered = true;

        // G5's legacy dispatcher treats [ClassName, method] as a singleton service
        // and calls ClassName::getInstance() before checking class_exists(). Closures
        // are the only PHP 8-safe core-free callback form for plain static handlers.
        add_event('bbs_write', static fn (array $board, int|string $writeId, string $mode): mixed =>
            Hooks::writeForm($board, $writeId, $mode), 10, 3);
        add_event('write_update_before', static fn (array $board, int|string $writeId, string $mode, string $query): mixed =>
            Hooks::beforeWrite($board, $writeId, $mode, $query), 10, 4);
        add_event('write_update_after', static fn (
            array $board,
            int|string $writeId,
            string $mode,
            string $query,
            string $redirectUrl,
        ): mixed => Hooks::afterWrite($board, $writeId, $mode, $query, $redirectUrl), 10, 5);
        add_replace('get_files', static fn (array $files, string $boardTable, int|string $writeId): array =>
            Hooks::filterFiles($files, $boardTable, $writeId), 10, 3);
        add_replace('download_file_exist_check', static fn (bool $exists, array $file): bool =>
            Hooks::remoteFileExists($exists, $file), 10, 2);
        add_event('download_file_header', static fn (array $file, bool $fileExists): mixed =>
            Hooks::downloadHeader($file, $fileExists), 10, 2);
        add_replace('delete_file_path', static fn (string $path, array $file): string =>
            Hooks::scheduleRemoteDeletion($path, $file), 10, 2);
    }
}
