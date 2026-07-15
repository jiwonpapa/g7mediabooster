<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5\Tests\Unit;

use Jiwonpapa\G7MediaBooster\Gnuboard5\HmacSigner;
use PHPUnit\Framework\Attributes\Test;
use PHPUnit\Framework\TestCase;

final class HmacSignerTest extends TestCase
{
    #[Test]
    public function signature_matches_the_rust_canonical_contract(): void
    {
        $headers = (new HmacSigner)->sign(
            'g5-site-1',
            str_repeat('s', 32),
            'POST',
            '/v1/upload-batches',
            '{"files":[]}',
            1_700_000_000,
            '0123456789abcdef0123456789abcdef',
        );

        self::assertSame('1700000000', $headers['x-g7mb-timestamp']);
        self::assertSame(hash('sha256', '{"files":[]}'), $headers['x-g7mb-content-sha256']);
        self::assertSame('J4KWC2tv4cVDqfZvF19rq0Rez0i9m6A8Xi5sFtyKcvQ', $headers['x-g7mb-signature']);
    }
}
