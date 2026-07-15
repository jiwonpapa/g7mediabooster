<?php

declare(strict_types=1);

if (PHP_SAPI !== 'cli') {
    fwrite(STDERR, "CLI only\n");
    exit(64);
}
$root = $argv[1] ?? getenv('G7MB_G5_ROOT');
if (! is_string($root) || ! is_file(rtrim($root, '/').'/common.php')) {
    fwrite(STDERR, "usage: php install.php /absolute/path/to/gnuboard5\n");
    exit(64);
}
$_SERVER['REMOTE_ADDR'] ??= '127.0.0.1';
$_SERVER['SERVER_ADDR'] ??= '127.0.0.1';
$_SERVER['SERVER_NAME'] ??= 'localhost';
$_SERVER['HTTP_HOST'] ??= 'localhost';
$_SERVER['SERVER_PORT'] ??= '80';
$_SERVER['REQUEST_URI'] ??= '/';
$_SERVER['SERVER_SOFTWARE'] ??= 'g7mediabooster-cli';
chdir($root);
require rtrim($root, '/').'/common.php';
require dirname(__DIR__).'/bootstrap.php';

(new \Jiwonpapa\G7MediaBooster\Gnuboard5\GnuboardRuntime)->store()->install();
fwrite(STDOUT, "G7MediaBooster Gnuboard5 table: PASS\n");
