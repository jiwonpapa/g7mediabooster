<?php

declare(strict_types=1);

// Isolated stock host can reuse installed dependencies without loading custom host source.
$root = getenv('G7MB_TEST_HOST_ROOT');
if (! is_string($root) || ! is_file($root.'/artisan')) {
    throw new RuntimeException('G7MB_TEST_HOST_ROOT must identify an isolated test host');
}
$loader = require $root.'/vendor/autoload.php';
$loader->setPsr4('App\\', [$root.'/app']);
$loader->setPsr4('Tests\\', [$root.'/tests']);
$loader->setPsr4('Modules\\Sirsoft\\Board\\', [$root.'/modules/_bundled/sirsoft-board/src']);
$_ENV['APP_BASE_PATH'] = $root;
$_SERVER['APP_BASE_PATH'] = $root;
require_once $root.'/modules/_bundled/jiwonpapa-g7mediabooster/tests/bootstrap.php';
