<?php

declare(strict_types=1);

use App\Extension\Helpers\ZipInstallHelper;
use Illuminate\Contracts\Console\Kernel;
use Illuminate\Support\Facades\File;

if ($argc !== 4) {
    fwrite(STDERR, "usage: php verify-gnuboard7-module-zip.php /path/to/gnuboard7 module.zip expected-version\n");
    exit(64);
}

$g7Root = rtrim($argv[1], DIRECTORY_SEPARATOR);
$archive = $argv[2];
$expectedVersion = $argv[3];
$autoload = $g7Root.'/vendor/autoload.php';
$bootstrap = $g7Root.'/bootstrap/app.php';

if (! is_file($autoload) || ! is_file($bootstrap) || ! is_file($archive)) {
    fwrite(STDERR, "Gnuboard7 bootstrap or module ZIP is missing\n");
    exit(66);
}
if (preg_match('/^[0-9]+\.[0-9]+\.[0-9]+$/', $expectedVersion) !== 1) {
    fwrite(STDERR, "expected version must be semantic version\n");
    exit(64);
}

require $autoload;
$app = require $bootstrap;
$app->make(Kernel::class)->bootstrap();
$temporary = sys_get_temp_dir().'/g7mb-module-zip-'.bin2hex(random_bytes(16));

try {
    $result = ZipInstallHelper::extractAndValidate($archive, $temporary, 'module.json', 'modules');
    $manifest = $result['config'] ?? null;
    if (($result['identifier'] ?? null) !== 'jiwonpapa-g7mediabooster' || ! is_array($manifest)) {
        throw new RuntimeException('G7 module identifier is invalid');
    }
    if (($manifest['version'] ?? null) !== $expectedVersion) {
        throw new RuntimeException('G7 module version does not match the expected release');
    }
    if (($manifest['dependencies']['modules']['sirsoft-board'] ?? null) !== '>=1.2.0') {
        throw new RuntimeException('G7 module sirsoft-board contract is invalid');
    }
    if (($manifest['compatibility']['contracts']['sirsoft-board.secure-external-attachments'] ?? null)
        !== '>=1.0.0 <2.0.0') {
        throw new RuntimeException('G7 module activation capability contract is invalid');
    }

    printf(
        "G7 ZipInstallHelper PASS identifier=%s version=%s\n",
        $result['identifier'],
        $manifest['version'],
    );
} finally {
    if (is_dir($temporary)) {
        File::deleteDirectory($temporary);
    }
}
