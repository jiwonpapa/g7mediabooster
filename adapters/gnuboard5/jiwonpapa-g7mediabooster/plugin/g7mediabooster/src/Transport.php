<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

interface Transport
{
    /** @param array<string, string> $headers */
    public function send(
        string $method,
        string $url,
        array $headers,
        string $body,
        int $connectTimeoutSeconds,
        int $timeoutSeconds,
    ): TransportResponse;
}
