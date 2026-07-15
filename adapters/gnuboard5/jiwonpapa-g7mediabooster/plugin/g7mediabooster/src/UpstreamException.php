<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use RuntimeException;

final class UpstreamException extends RuntimeException
{
    public function __construct(
        public readonly int $httpStatus,
        public readonly string $errorCode,
        string $message,
        public readonly ?string $requestId = null,
    ) {
        parent::__construct($message);
    }
}
