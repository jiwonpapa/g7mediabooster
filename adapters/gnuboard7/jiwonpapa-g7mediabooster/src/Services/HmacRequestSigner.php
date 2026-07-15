<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use InvalidArgumentException;

final class HmacRequestSigner
{
    /**
     * @return array<string, string>
     */
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

        $this->assertFields($keyId, $secret, $method, $pathAndQuery, $nonce, $bodyHash);
        $canonical = $this->canonicalPayload(
            $keyId,
            $timestamp,
            $nonce,
            $method,
            $pathAndQuery,
            $bodyHash,
        );
        $signature = rtrim(strtr(base64_encode(hash_hmac('sha256', $canonical, $secret, true)), '+/', '-_'), '=');

        return [
            'x-g7mb-key-id' => $keyId,
            'x-g7mb-timestamp' => (string) $timestamp,
            'x-g7mb-nonce' => $nonce,
            'x-g7mb-content-sha256' => $bodyHash,
            'x-g7mb-signature' => $signature,
        ];
    }

    public function canonicalPayload(
        string $keyId,
        int $timestamp,
        string $nonce,
        string $method,
        string $pathAndQuery,
        string $bodyHash,
    ): string {
        return implode("\n", [
            'G7MB-HMAC-SHA256',
            $keyId,
            (string) $timestamp,
            $nonce,
            $method,
            $pathAndQuery,
            $bodyHash,
        ]);
    }

    private function assertFields(
        string $keyId,
        string $secret,
        string $method,
        string $pathAndQuery,
        string $nonce,
        string $bodyHash,
    ): void {
        if (! preg_match('/^[A-Za-z0-9_-]{1,128}$/', $keyId)) {
            throw new InvalidArgumentException('invalid HMAC key id');
        }
        if (strlen($secret) < 32 || strlen($secret) > 256) {
            throw new InvalidArgumentException('invalid HMAC secret length');
        }
        if (! preg_match('/^[A-Z]+$/', $method)) {
            throw new InvalidArgumentException('invalid HTTP method');
        }
        if (! preg_match('/^\/[\x21-\x7e]{0,8191}$/', $pathAndQuery)) {
            throw new InvalidArgumentException('invalid path and query');
        }
        if (strlen($nonce) < 16 || strlen($nonce) > 128 || ! preg_match('/^[\x21-\x7e]+$/', $nonce)) {
            throw new InvalidArgumentException('invalid request nonce');
        }
        if (! preg_match('/^[a-f0-9]{64}$/', $bodyHash)) {
            throw new InvalidArgumentException('invalid request body digest');
        }
    }
}
