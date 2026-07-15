<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Exceptions;

use RuntimeException;

final class MediaBoosterUpstreamException extends RuntimeException
{
    public function __construct(
        public readonly int $httpStatus,
        public readonly string $errorCode,
        string $safeMessage,
        public readonly ?string $requestId = null,
    ) {
        parent::__construct($safeMessage);
    }
}
