<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use InvalidArgumentException;

final class HmacSigner
{
    /** @return array<string, string> */
    public function sign(
        string $keyId,
        string $secret,
        string $method,
        string $pathAndQuery,
        string $body,
        ?int $timestamp = null,
        ?string $nonce = null,
    ): array {
        $method = strtoupper($method);
        $timestamp ??= time();
        $nonce ??= bin2hex(random_bytes(24));
        $bodyHash = hash('sha256', $body);
        if (! preg_match('/^[A-Za-z0-9_-]{1,128}$/', $keyId)
            || strlen($secret) < 32
            || strlen($secret) > 256
            || ! preg_match('/^[A-Z]+$/', $method)
            || ! preg_match('/^\/[\x21-\x7e]{0,8191}$/', $pathAndQuery)
            || strlen($nonce) < 16
            || strlen($nonce) > 128
            || ! preg_match('/^[\x21-\x7e]+$/', $nonce)
        ) {
            throw new InvalidArgumentException('invalid HMAC signing input');
        }

        $canonical = implode("\n", [
            'G7MB-HMAC-SHA256',
            $keyId,
            (string) $timestamp,
            $nonce,
            $method,
            $pathAndQuery,
            $bodyHash,
        ]);
        $signature = rtrim(strtr(base64_encode(hash_hmac('sha256', $canonical, $secret, true)), '+/', '-_'), '=');

        return [
            'x-g7mb-key-id' => $keyId,
            'x-g7mb-timestamp' => (string) $timestamp,
            'x-g7mb-nonce' => $nonce,
            'x-g7mb-content-sha256' => $bodyHash,
            'x-g7mb-signature' => $signature,
        ];
    }
}
