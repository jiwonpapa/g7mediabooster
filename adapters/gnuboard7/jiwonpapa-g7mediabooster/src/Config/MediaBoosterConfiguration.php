<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Config;

use InvalidArgumentException;

final readonly class MediaBoosterConfiguration
{
    private const LOOPBACK_HOSTS = ['localhost', '127.0.0.1', '::1'];

    private function __construct(
        public bool $enabled,
        public string $endpoint,
        public string $keyId,
        public string $hmacSecret,
        public int $timeoutSeconds,
        public int $connectTimeoutSeconds,
        public int $maxParallelFiles,
        public int $maxParallelParts,
        public int $maxPartRetries,
        public int $statusPollIntervalMs,
        public bool $watermarkEnabled,
        public string $watermarkAssetUploadId,
        public string $watermarkPosition,
        public int $watermarkMarginPx,
        public int $watermarkMaxWidthPercent,
        public int $watermarkOpacityPercent,
    ) {}

    /**
     * @param array<string, mixed> $settings
     */
    public static function fromArray(array $settings): self
    {
        $enabled = filter_var($settings['enabled'] ?? false, FILTER_VALIDATE_BOOL);
        $endpoint = self::validatedEndpoint((string) ($settings['control_endpoint'] ?? ''));
        $keyId = (string) ($settings['key_id'] ?? '');
        $secret = (string) ($settings['hmac_secret'] ?? '');

        if (! preg_match('/^[A-Za-z0-9_-]{1,128}$/', $keyId)) {
            throw new InvalidArgumentException('key_id must use 1-128 ASCII letters, digits, hyphens, or underscores');
        }

        $secretLength = strlen($secret);
        if ($enabled && ($secretLength < 32 || $secretLength > 256)) {
            throw new InvalidArgumentException('hmac_secret must contain 32-256 bytes when enabled');
        }
        if (! $enabled && $secret !== '' && ($secretLength < 32 || $secretLength > 256)) {
            throw new InvalidArgumentException('hmac_secret must be empty or contain 32-256 bytes');
        }

        $timeout = self::boundedInt($settings, 'timeout_seconds', 15, 1, 60);
        $connectTimeout = self::boundedInt($settings, 'connect_timeout_seconds', 3, 1, 15);
        if ($connectTimeout > $timeout) {
            throw new InvalidArgumentException('connect_timeout_seconds cannot exceed timeout_seconds');
        }

        $watermarkEnabled = filter_var($settings['watermark_enabled'] ?? false, FILTER_VALIDATE_BOOL);
        $watermarkAssetUploadId = trim((string) ($settings['watermark_asset_upload_id'] ?? ''));
        if ($watermarkEnabled && ! self::isUploadId($watermarkAssetUploadId)) {
            throw new InvalidArgumentException('watermark_asset_upload_id must be a valid Ready image upload UUID');
        }
        if (! $watermarkEnabled && $watermarkAssetUploadId !== '' && ! self::isUploadId($watermarkAssetUploadId)) {
            throw new InvalidArgumentException('watermark_asset_upload_id must be empty or a valid upload UUID');
        }
        $watermarkPosition = (string) ($settings['watermark_position'] ?? 'bottom_right');
        if (! in_array($watermarkPosition, ['center', 'top_left', 'top_right', 'bottom_left', 'bottom_right'], true)) {
            throw new InvalidArgumentException('watermark_position is not allowlisted');
        }

        return new self(
            enabled: $enabled,
            endpoint: $endpoint,
            keyId: $keyId,
            hmacSecret: $secret,
            timeoutSeconds: $timeout,
            connectTimeoutSeconds: $connectTimeout,
            maxParallelFiles: self::boundedInt($settings, 'max_parallel_files', 8, 1, 16),
            maxParallelParts: self::boundedInt($settings, 'max_parallel_parts', 4, 1, 8),
            maxPartRetries: self::boundedInt($settings, 'max_part_retries', 3, 0, 5),
            statusPollIntervalMs: self::boundedInt($settings, 'status_poll_interval_ms', 1500, 1500, 10_000),
            watermarkEnabled: $watermarkEnabled,
            watermarkAssetUploadId: $watermarkAssetUploadId,
            watermarkPosition: $watermarkPosition,
            watermarkMarginPx: self::boundedInt($settings, 'watermark_margin_px', 24, 0, 1024),
            watermarkMaxWidthPercent: self::boundedInt($settings, 'watermark_max_width_percent', 20, 1, 50),
            watermarkOpacityPercent: self::boundedInt($settings, 'watermark_opacity_percent', 80, 1, 100),
        );
    }

    private static function validatedEndpoint(string $endpoint): string
    {
        if ($endpoint === '' || trim($endpoint) !== $endpoint || preg_match('/[\x00-\x20\x7f]/', $endpoint)) {
            throw new InvalidArgumentException('control_endpoint is invalid');
        }

        $parts = parse_url($endpoint);
        if (! is_array($parts)) {
            throw new InvalidArgumentException('control_endpoint is invalid');
        }

        $scheme = strtolower((string) ($parts['scheme'] ?? ''));
        $host = strtolower(trim((string) ($parts['host'] ?? ''), '[]'));
        $path = (string) ($parts['path'] ?? '');
        if ($host === '' || ! in_array($scheme, ['http', 'https'], true)) {
            throw new InvalidArgumentException('control_endpoint must be an HTTP(S) origin');
        }
        if (isset($parts['user']) || isset($parts['pass']) || isset($parts['query']) || isset($parts['fragment'])) {
            throw new InvalidArgumentException('control_endpoint must not contain credentials, query, or fragment');
        }
        if ($path !== '' && $path !== '/') {
            throw new InvalidArgumentException('control_endpoint must not contain a path');
        }
        if ($scheme === 'http' && ! self::isLoopbackHost($host)) {
            throw new InvalidArgumentException('plain HTTP is allowed only for a literal loopback host');
        }
        if (isset($parts['port']) && ($parts['port'] < 1 || $parts['port'] > 65_535)) {
            throw new InvalidArgumentException('control_endpoint port is invalid');
        }

        return rtrim($endpoint, '/');
    }

    private static function isLoopbackHost(string $host): bool
    {
        if (in_array($host, self::LOOPBACK_HOSTS, true)) {
            return true;
        }

        if (filter_var($host, FILTER_VALIDATE_IP, FILTER_FLAG_IPV4)) {
            return str_starts_with($host, '127.');
        }

        return false;
    }

    private static function isUploadId(string $uploadId): bool
    {
        return preg_match('/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/', $uploadId) === 1;
    }

    /**
     * @param array<string, mixed> $settings
     */
    private static function boundedInt(array $settings, string $key, int $default, int $min, int $max): int
    {
        $value = filter_var($settings[$key] ?? $default, FILTER_VALIDATE_INT);
        if (! is_int($value) || $value < $min || $value > $max) {
            throw new InvalidArgumentException("{$key} must be between {$min} and {$max}");
        }

        return $value;
    }
}
