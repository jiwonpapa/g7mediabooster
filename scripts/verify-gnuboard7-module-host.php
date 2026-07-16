<?php

declare(strict_types=1);

use Modules\Jiwonpapa\G7mediabooster\Compatibility\Gnuboard7MediaContract;

if ($argc !== 3) {
    fwrite(STDERR, "usage: php verify-gnuboard7-module-host.php /path/to/gnuboard7 /path/to/module\n");
    exit(64);
}

$g7Root = realpath($argv[1]);
$moduleRoot = realpath($argv[2]);
if ($g7Root === false || $moduleRoot === false) {
    fwrite(STDERR, "Gnuboard7 or module root is invalid\n");
    exit(66);
}

$prefixes = [
    'App\\' => $g7Root.'/app/',
    'Modules\\Sirsoft\\Board\\' => $g7Root.'/modules/_bundled/sirsoft-board/src/',
    'Modules\\Jiwonpapa\\G7mediabooster\\' => $moduleRoot.'/src/',
];
spl_autoload_register(static function (string $class) use ($prefixes): void {
    foreach ($prefixes as $prefix => $root) {
        if (! str_starts_with($class, $prefix)) {
            continue;
        }
        $path = $root.str_replace('\\', '/', substr($class, strlen($prefix))).'.php';
        if (is_file($path)) {
            require $path;
        }

        return;
    }
});

try {
    Gnuboard7MediaContract::assertCompatible($g7Root);
    require_once $moduleRoot.'/module.php';
    $moduleClass = 'Modules\\Jiwonpapa\\G7mediabooster\\Module';
    $module = new $moduleClass;
    if ($module->activate() !== true) {
        throw new RuntimeException('G7MB_G7_MODULE_ACTIVATION_REJECTED');
    }
} catch (Throwable $error) {
    fwrite(STDERR, 'G7 module activation contract: FAIL '.$error->getMessage()."\n");
    exit(1);
}

fwrite(STDOUT, "G7 module activation contract: PASS id=".Gnuboard7MediaContract::CONTRACT_ID.
    ' version='.Gnuboard7MediaContract::CONTRACT_VERSION."\n");
