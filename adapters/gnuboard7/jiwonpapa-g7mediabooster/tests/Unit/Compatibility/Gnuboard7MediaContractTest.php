<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Compatibility;

use Modules\Jiwonpapa\G7mediabooster\Compatibility\Gnuboard7MediaContract;
use PHPUnit\Framework\TestCase;

final class Gnuboard7MediaContractTest extends TestCase
{
    public function test_real_stock_host_is_accepted_without_external_media_patch(): void
    {
        $root = getenv('G7MB_TEST_HOST_ROOT');
        if (! is_string($root) || $root === '') {
            self::markTestSkipped('Stock G7 source is required.');
        }
        self::assertFileDoesNotExist($root.'/modules/_bundled/sirsoft-board/resources/contracts/external-media-v1.json');
        Gnuboard7MediaContract::assertCompatible($root);
        self::addToAssertionCount(1);
    }
}
