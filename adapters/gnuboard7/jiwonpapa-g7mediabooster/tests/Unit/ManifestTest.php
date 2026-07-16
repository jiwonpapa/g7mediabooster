<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit;

use PHPUnit\Framework\TestCase;

final class ManifestTest extends TestCase
{
    public function testManifestAndSensitiveSettingsBoundary(): void
    {
        $root = dirname(__DIR__, 2);
        $manifest = json_decode((string) file_get_contents($root.'/module.json'), true, flags: JSON_THROW_ON_ERROR);
        $defaults = json_decode((string) file_get_contents($root.'/config/settings/defaults.json'), true, flags: JSON_THROW_ON_ERROR);

        self::assertSame('jiwonpapa-g7mediabooster', $manifest['identifier']);
        self::assertSame('0.4.1', $manifest['version']);
        self::assertSame('>=1.2.0', $manifest['dependencies']['modules']['sirsoft-board']);
        self::assertArrayHasKey('hmac_secret', $defaults['defaults']);
        self::assertSame('http://127.0.0.1:8088', $defaults['defaults']['control_endpoint']);
        self::assertSame(30, $defaults['defaults']['attachment_retention_days']);
        self::assertStringNotContainsString('hmac_secret', json_encode($defaults['frontend_schema'], JSON_THROW_ON_ERROR));
    }
}
