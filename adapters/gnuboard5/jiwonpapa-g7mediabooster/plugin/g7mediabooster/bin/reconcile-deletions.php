<?php

declare(strict_types=1);

if (PHP_SAPI !== 'cli') {
    fwrite(STDERR, "CLI only\n");
    exit(64);
}
$root = $argv[1] ?? getenv('G7MB_G5_ROOT');
if (! is_string($root) || ! is_file(rtrim($root, '/').'/common.php')) {
    fwrite(STDERR, "usage: php reconcile-deletions.php /absolute/path/to/gnuboard5\n");
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

$lockPath = G5_DATA_PATH.'/g7mediabooster-delete.lock';
$lock = fopen($lockPath, 'c');
if ($lock === false || ! flock($lock, LOCK_EX | LOCK_NB)) {
    fwrite(STDOUT, "G7MediaBooster deletion reconciler already running\n");
    exit(0);
}
$runtime = new \Jiwonpapa\G7MediaBooster\Gnuboard5\GnuboardRuntime;
$store = $runtime->store();
$completed = 0;
$failed = 0;
foreach ($store->dueDeletions(100) as $session) {
    $uploadId = (string) $session['upload_id'];
    try {
        $runtime->client()->deleteUpload($uploadId);
        $store->completeDeletionRequest($uploadId);
        $completed++;
    } catch (\Jiwonpapa\G7MediaBooster\Gnuboard5\UpstreamException $error) {
        $store->failDeletionRequest($uploadId, $error->errorCode);
        $failed++;
    } catch (Throwable) {
        $store->failDeletionRequest($uploadId, 'DELETE_REQUEST_FAILED');
        $failed++;
    }
}
fwrite(STDOUT, "G7MediaBooster deletion reconciler: completed={$completed} failed={$failed}\n");
exit($failed > 0 ? 1 : 0);
