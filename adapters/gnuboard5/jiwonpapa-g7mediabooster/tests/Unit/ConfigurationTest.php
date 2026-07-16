<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5\Tests\Unit;

use InvalidArgumentException;
use Jiwonpapa\G7MediaBooster\Gnuboard5\Configuration;
use PHPUnit\Framework\Attributes\Test;
use PHPUnit\Framework\TestCase;

final class ConfigurationTest extends TestCase
{
    #[Test]
    public function disabled_configuration_does_not_require_secrets(): void
    {
        $configuration = Configuration::fromEnvironment(['G7MB_G5_ENABLED' => 'false']);

        self::assertFalse($configuration->enabled);
        self::assertSame('http://127.0.0.1:8088', $configuration->endpoint);
        self::assertSame(8, $configuration->maxParallelFiles);
    }

    #[Test]
    public function enabled_configuration_accepts_https_and_bounded_values(): void
    {
        $configuration = Configuration::fromEnvironment([
            'G7MB_G5_ENABLED' => 'true',
            'G7MB_G5_ENDPOINT' => 'https://media.example.com',
            'G7MB_G5_KEY_ID' => 'g5-site-1',
            'G7MB_G5_HMAC_SECRET' => str_repeat('s', 32),
            'G7MB_G5_MAX_PARALLEL_FILES' => '6',
        ]);

        self::assertTrue($configuration->enabled);
        self::assertSame(6, $configuration->maxParallelFiles);
    }

    #[Test]
    public function enabled_configuration_rejects_remote_plain_http(): void
    {
        $this->expectException(InvalidArgumentException::class);

        Configuration::fromEnvironment([
            'G7MB_G5_ENABLED' => 'true',
            'G7MB_G5_ENDPOINT' => 'http://media.example.com',
            'G7MB_G5_KEY_ID' => 'g5-site-1',
            'G7MB_G5_HMAC_SECRET' => str_repeat('s', 32),
        ]);
    }
}
