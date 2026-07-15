<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use DateTimeImmutable;
use UnexpectedValueException;

final class DerivativeDeliveryValidator
{
    private const LOOPBACK_HOSTS = ['localhost', '127.0.0.1', '::1'];

    /**
     * @param array<string, mixed> $delivery
     */
    public function validate(array $delivery, string $uploadId, string $variant): string
    {
        if (! in_array($variant, ['master', 'thumbnail'], true)
            || ! is_string($delivery['upload_id'] ?? null)
            || strtolower($delivery['upload_id']) !== strtolower($uploadId)
            || ($delivery['variant'] ?? null) !== $variant
            || ! is_string($delivery['preset_id'] ?? null)
            || preg_match('/^[A-Za-z0-9._-]{1,160}$/', $delivery['preset_id']) !== 1
            || ! is_string($delivery['content_type'] ?? null)
            || preg_match('#^(?:image|video)/[a-z0-9.+-]{1,120}$#', $delivery['content_type']) !== 1
            || ! is_int($delivery['byte_len'] ?? null)
            || $delivery['byte_len'] < 1
            || ! is_string($delivery['expires_at'] ?? null)
            || ! $this->isFutureTimestamp($delivery['expires_at'])
        ) {
            throw new UnexpectedValueException('invalid derivative delivery response');
        }

        $url = $delivery['delivery_url'] ?? null;
        if (! is_string($url)
            || $url === ''
            || strlen($url) > 8192
            || trim($url) !== $url
            || preg_match('/[\x00-\x20\x7f]/', $url)
        ) {
            throw new UnexpectedValueException('invalid derivative delivery URL');
        }
        $parts = parse_url($url);
        if (! is_array($parts)
            || ! is_string($parts['scheme'] ?? null)
            || ! is_string($parts['host'] ?? null)
            || $parts['host'] === ''
            || isset($parts['user'])
            || isset($parts['pass'])
            || isset($parts['fragment'])
        ) {
            throw new UnexpectedValueException('invalid derivative delivery URL');
        }
        $scheme = strtolower($parts['scheme']);
        $host = strtolower(trim($parts['host'], '[]'));
        if ($scheme !== 'https' && ! ($scheme === 'http' && $this->isLoopback($host))) {
            throw new UnexpectedValueException('insecure derivative delivery URL');
        }

        return $url;
    }

    /**
     * @param array<string, mixed> $delivery
     */
    public function validateExact(
        array $delivery,
        string $uploadId,
        string $variant,
        string $expectedPresetId,
        string $expectedContentType,
        int $expectedByteLen,
    ): string {
        $url = $this->validate($delivery, $uploadId, $variant);
        if (($delivery['preset_id'] ?? null) !== $expectedPresetId
            || ($delivery['content_type'] ?? null) !== $expectedContentType
            || ($delivery['byte_len'] ?? null) !== $expectedByteLen
        ) {
            throw new UnexpectedValueException('derivative no longer matches the native attachment contract');
        }

        return $url;
    }

    private function isFutureTimestamp(string $value): bool
    {
        if (preg_match('/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?(?:Z|[+-]\d{2}:\d{2})$/', $value) !== 1) {
            return false;
        }
        try {
            $expiresAt = new DateTimeImmutable($value);
        } catch (\Exception) {
            return false;
        }

        return $expiresAt->getTimestamp() > time();
    }

    private function isLoopback(string $host): bool
    {
        if (in_array($host, self::LOOPBACK_HOSTS, true)) {
            return true;
        }

        return filter_var($host, FILTER_VALIDATE_IP, FILTER_FLAG_IPV4) !== false
            && str_starts_with($host, '127.');
    }
}
