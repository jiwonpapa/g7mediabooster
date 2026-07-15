<?php

declare(strict_types=1);

if (! defined('_GNUBOARD_')) {
    exit;
}

$g7mbBootstrap = G5_PLUGIN_PATH.'/g7mediabooster/bootstrap.php';
if (is_file($g7mbBootstrap)) {
    require_once $g7mbBootstrap;
    \Jiwonpapa\G7MediaBooster\Gnuboard5\Plugin::register();
}
