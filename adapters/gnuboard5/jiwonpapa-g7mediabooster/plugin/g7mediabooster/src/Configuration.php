<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use InvalidArgumentException;

final class Configuration
{
    public function __construct(
        public readonly bool $enabled,
        public readonly string $endpoint,
        public readonly string $keyId,
        public readonly string $hmacSecret,
        public readonly int $connectTimeoutSeconds = 2,
        public readonly int $timeoutSeconds = 10,
        public readonly int $maxParallelFiles = 8,
        public readonly int $maxParallelParts = 4,
        public readonly int $maxPartRetries = 3,
        public readonly int $statusPollIntervalMs = 1500,
        public readonly int $deleteRetentionDays = 7,
    ) {
        if (! $enabled) {
            return;
        }

        $parts = parse_url($endpoint);
        $host = strtolower((string) ($parts['host'] ?? ''));
        $scheme = strtolower((string) ($parts['scheme'] ?? ''));
        $loopback = in_array($host, ['127.0.0.1', '::1', 'localhost'], true);
        if ($endpoint !== rtrim($endpoint, '/')
            || ($scheme !== 'https' && ! ($scheme === 'http' && $loopback))
            || isset($parts['user'])
            || isset($parts['pass'])
            || isset($parts['query'])
            || isset($parts['fragment'])
            || $host === ''
        ) {
            throw new InvalidArgumentException('G7MB_G5_ENDPOINT must be HTTPS or loopback HTTP without credentials or query');
        }
        if (! preg_match('/^[A-Za-z0-9_-]{1,128}$/', $keyId)) {
            throw new InvalidArgumentException('G7MB_G5_KEY_ID is invalid');
        }
        if (strlen($hmacSecret) < 32 || strlen($hmacSecret) > 256) {
            throw new InvalidArgumentException('G7MB_G5_HMAC_SECRET must be 32-256 bytes');
        }
        self::bounded($connectTimeoutSeconds, 1, 10, 'connect timeout');
        self::bounded($timeoutSeconds, 2, 30, 'request timeout');
        self::bounded($maxParallelFiles, 1, 16, 'parallel files');
        self::bounded($maxParallelParts, 1, 8, 'parallel parts');
        self::bounded($maxPartRetries, 0, 5, 'part retries');
        self::bounded($statusPollIntervalMs, 1500, 30_000, 'poll interval');
        self::bounded($deleteRetentionDays, 0, 365, 'delete retention days');
    }

    /** @param array<string, string|false|null> $environment */
    public static function fromEnvironment(array $environment = []): self
    {
        $read = static function (string $name, string $default = '') use ($environment): string {
            $value = array_key_exists($name, $environment) ? $environment[$name] : getenv($name);

            return is_string($value) ? trim($value) : $default;
        };
        $enabled = filter_var($read('G7MB_G5_ENABLED', 'false'), FILTER_VALIDATE_BOOL);

        return new self(
            enabled: $enabled,
            endpoint: rtrim($read('G7MB_G5_ENDPOINT', 'http://127.0.0.1:8080'), '/'),
            keyId: $read('G7MB_G5_KEY_ID', 'g5-disabled'),
            hmacSecret: $read('G7MB_G5_HMAC_SECRET'),
            connectTimeoutSeconds: self::integer($read('G7MB_G5_CONNECT_TIMEOUT_SECONDS', '2'), 2),
            timeoutSeconds: self::integer($read('G7MB_G5_TIMEOUT_SECONDS', '10'), 10),
            maxParallelFiles: self::integer($read('G7MB_G5_MAX_PARALLEL_FILES', '8'), 8),
            maxParallelParts: self::integer($read('G7MB_G5_MAX_PARALLEL_PARTS', '4'), 4),
            maxPartRetries: self::integer($read('G7MB_G5_MAX_PART_RETRIES', '3'), 3),
            statusPollIntervalMs: self::integer($read('G7MB_G5_STATUS_POLL_INTERVAL_MS', '1500'), 1500),
            deleteRetentionDays: self::integer($read('G7MB_G5_DELETE_RETENTION_DAYS', '7'), 7),
        );
    }

    private static function integer(string $value, int $default): int
    {
        return preg_match('/^-?[0-9]+$/', $value) ? (int) $value : $default;
    }

    private static function bounded(int $value, int $minimum, int $maximum, string $name): void
    {
        if ($value < $minimum || $value > $maximum) {
            throw new InvalidArgumentException("G7MediaBooster {$name} is outside the hard bound");
        }
    }
}
