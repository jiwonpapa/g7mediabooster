<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Config;

use InvalidArgumentException;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

final class MediaBoosterConfigurationTest extends TestCase
{
    public function testEmptySettingsLoadSafeDisabledDefaultsDuringFirstActivation(): void
    {
        $configuration = MediaBoosterConfiguration::fromArray([]);

        self::assertFalse($configuration->enabled);
        self::assertSame('http://127.0.0.1:8080', $configuration->endpoint);
        self::assertSame('g7-primary', $configuration->keyId);
        self::assertSame('', $configuration->hmacSecret);
    }

    public function testAcceptsSecureOriginAndBoundedUploaderSettings(): void
    {
        $configuration = MediaBoosterConfiguration::fromArray($this->validSettings());

        self::assertTrue($configuration->enabled);
        self::assertSame('https://media-control.example.com:8443', $configuration->endpoint);
        self::assertSame(8, $configuration->maxParallelFiles);
        self::assertSame(4, $configuration->maxParallelParts);
        self::assertSame(30, $configuration->attachmentRetentionDays);
        self::assertFalse($configuration->watermarkEnabled);
    }

    public function testDisabledConfigurationMayHaveNoSecret(): void
    {
        $settings = $this->validSettings();
        $settings['enabled'] = false;
        $settings['hmac_secret'] = '';
        $settings['control_endpoint'] = 'http://127.12.3.4:8080/';

        $configuration = MediaBoosterConfiguration::fromArray($settings);

        self::assertFalse($configuration->enabled);
        self::assertSame('http://127.12.3.4:8080', $configuration->endpoint);
    }

    /** @return iterable<string, array{string}> */
    public static function unsafeEndpoints(): iterable
    {
        yield 'plain remote HTTP' => ['http://media.example.com'];
        yield 'userinfo' => ['https://user:pass@media.example.com'];
        yield 'query' => ['https://media.example.com?target=other'];
        yield 'fragment' => ['https://media.example.com/#admin'];
        yield 'path prefix' => ['https://media.example.com/internal'];
        yield 'loopback lookalike' => ['http://127.0.0.1.example.com'];
    }

    #[DataProvider('unsafeEndpoints')]
    public function testRejectsUnsafeControlEndpoints(string $endpoint): void
    {
        $settings = $this->validSettings();
        $settings['control_endpoint'] = $endpoint;

        $this->expectException(InvalidArgumentException::class);
        MediaBoosterConfiguration::fromArray($settings);
    }

    public function testRejectsEnabledConfigurationWithShortSecret(): void
    {
        $settings = $this->validSettings();
        $settings['hmac_secret'] = 'short';

        $this->expectException(InvalidArgumentException::class);
        MediaBoosterConfiguration::fromArray($settings);
    }

    public function testRejectsAPollRateThatCanExhaustTheUserRouteBudget(): void
    {
        $settings = $this->validSettings();
        $settings['status_poll_interval_ms'] = 1499;

        $this->expectException(InvalidArgumentException::class);
        MediaBoosterConfiguration::fromArray($settings);
    }

    /** @return array<string, mixed> */
    private function validSettings(): array
    {
        return [
            'enabled' => true,
            'control_endpoint' => 'https://media-control.example.com:8443',
            'key_id' => 'g7-primary',
            'hmac_secret' => '0123456789abcdef0123456789abcdef',
            'timeout_seconds' => 15,
            'connect_timeout_seconds' => 3,
            'max_parallel_files' => 8,
            'max_parallel_parts' => 4,
            'max_part_retries' => 3,
            'status_poll_interval_ms' => 1500,
            'attachment_retention_days' => 30,
            'watermark_enabled' => false,
            'watermark_asset_upload_id' => '',
            'watermark_position' => 'bottom_right',
            'watermark_margin_px' => 24,
            'watermark_max_width_percent' => 20,
            'watermark_opacity_percent' => 80,
        ];
    }
}
