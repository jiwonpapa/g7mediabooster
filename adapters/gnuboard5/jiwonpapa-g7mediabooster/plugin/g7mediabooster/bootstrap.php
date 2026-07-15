<?php

declare(strict_types=1);

if (! defined('_GNUBOARD_')) {
    exit;
}

spl_autoload_register(static function (string $class): void {
    $prefix = 'Jiwonpapa\\G7MediaBooster\\Gnuboard5\\';
    if (! str_starts_with($class, $prefix)) {
        return;
    }
    $relative = str_replace('\\', '/', substr($class, strlen($prefix)));
    $path = __DIR__.'/src/'.$relative.'.php';
    if (is_file($path)) {
        require_once $path;
    }
});

global $g5;
if (defined('G5_TABLE_PREFIX')
    && is_string(G5_TABLE_PREFIX)
    && preg_match('/^[A-Za-z0-9_]+$/', G5_TABLE_PREFIX)
) {
    $g5['g7mb_upload_session_table'] = G5_TABLE_PREFIX.'g7mb_upload_sessions';
}
