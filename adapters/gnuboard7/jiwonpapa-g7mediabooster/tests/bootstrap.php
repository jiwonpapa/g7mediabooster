<?php

declare(strict_types=1);

$moduleRoot = dirname(__DIR__);

spl_autoload_register(static function (string $class) use ($moduleRoot): void {
    $prefix = 'Modules\\Jiwonpapa\\G7mediabooster\\';
    if (! str_starts_with($class, $prefix)) {
        return;
    }

    $relative = substr($class, strlen($prefix));
    if (str_starts_with($relative, 'Tests\\')) {
        $path = $moduleRoot.'/tests/'.str_replace('\\', '/', substr($relative, strlen('Tests\\'))).'.php';
    } else {
        $path = $moduleRoot.'/src/'.str_replace('\\', '/', $relative).'.php';
    }

    if (is_file($path)) {
        require $path;
    }
});
