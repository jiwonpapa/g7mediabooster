<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5\Tests\Unit;

use PHPUnit\Framework\TestCase;

final class BootstrapTest extends TestCase
{
    public function testItUsesTheCanonicalGnuboardTablePrefixConstant(): void
    {
        if (! defined('_GNUBOARD_')) {
            define('_GNUBOARD_', true);
        }
        if (! defined('G5_TABLE_PREFIX')) {
            define('G5_TABLE_PREFIX', 'contract_');
        }
        global $g5;
        $g5 = [];

        require dirname(__DIR__, 2).'/plugin/g7mediabooster/bootstrap.php';

        self::assertSame('contract_g7mb_upload_sessions', $g5['g7mb_upload_session_table']);
    }
}
