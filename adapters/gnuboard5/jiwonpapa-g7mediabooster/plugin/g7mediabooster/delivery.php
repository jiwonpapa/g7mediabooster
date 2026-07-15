<?php

declare(strict_types=1);

require_once dirname(__DIR__, 2).'/common.php';
require_once __DIR__.'/bootstrap.php';

(new \Jiwonpapa\G7MediaBooster\Gnuboard5\DeliveryEndpoint(
    new \Jiwonpapa\G7MediaBooster\Gnuboard5\GnuboardRuntime,
))->run();
