<?php

declare(strict_types=1);

$root = dirname(__DIR__);
$autoload = $root.'/vendor/autoload.php';
if (is_file($autoload)) {
    require $autoload;

    return;
}

spl_autoload_register(static function (string $class) use ($root): void {
    $prefix = 'Jiwonpapa\\G7MediaBooster\\Gnuboard5\\';
    if (! str_starts_with($class, $prefix)) {
        return;
    }

    $relative = str_replace('\\', '/', substr($class, strlen($prefix)));
    $path = $root.'/plugin/g7mediabooster/src/'.$relative.'.php';
    if (is_file($path)) {
        require $path;
    }
});
