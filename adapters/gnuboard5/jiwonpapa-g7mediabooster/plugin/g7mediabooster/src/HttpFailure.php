<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use RuntimeException;

final class HttpFailure extends RuntimeException
{
    public function __construct(
        public readonly int $status,
        public readonly string $errorCode,
        string $message,
    ) {
        parent::__construct($message);
    }
}
