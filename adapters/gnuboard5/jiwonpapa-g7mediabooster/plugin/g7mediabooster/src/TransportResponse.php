<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

final class TransportResponse
{
    public function __construct(
        public readonly int $status,
        public readonly string $body,
    ) {}
}
