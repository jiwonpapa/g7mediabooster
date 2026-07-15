<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use RuntimeException;

final class CurlTransport implements Transport
{
    private const MAX_RESPONSE_BYTES = 1024 * 1024;

    public function send(
        string $method,
        string $url,
        array $headers,
        string $body,
        int $connectTimeoutSeconds,
        int $timeoutSeconds,
    ): TransportResponse {
        if (! function_exists('curl_init')) {
            throw new RuntimeException('PHP cURL extension is required');
        }

        $handle = curl_init($url);
        if ($handle === false) {
            throw new RuntimeException('cannot initialize control request');
        }
        $responseBody = '';
        $headerLines = [];
        foreach ($headers as $name => $value) {
            $headerLines[] = $name.': '.$value;
        }
        curl_setopt_array($handle, [
            CURLOPT_CUSTOMREQUEST => $method,
            CURLOPT_POSTFIELDS => $body,
            CURLOPT_HTTPHEADER => $headerLines,
            CURLOPT_RETURNTRANSFER => false,
            CURLOPT_FOLLOWLOCATION => false,
            CURLOPT_CONNECTTIMEOUT => $connectTimeoutSeconds,
            CURLOPT_TIMEOUT => $timeoutSeconds,
            CURLOPT_SSL_VERIFYPEER => true,
            CURLOPT_SSL_VERIFYHOST => 2,
            CURLOPT_PROTOCOLS => CURLPROTO_HTTP | CURLPROTO_HTTPS,
            CURLOPT_REDIR_PROTOCOLS => 0,
            CURLOPT_WRITEFUNCTION => static function ($curl, string $chunk) use (&$responseBody): int {
                if (strlen($responseBody) + strlen($chunk) > self::MAX_RESPONSE_BYTES) {
                    return 0;
                }
                $responseBody .= $chunk;

                return strlen($chunk);
            },
        ]);

        if (curl_exec($handle) === false) {
            throw new RuntimeException('control request failed');
        }
        $status = (int) curl_getinfo($handle, CURLINFO_RESPONSE_CODE);

        return new TransportResponse($status, $responseBody);
    }
}
