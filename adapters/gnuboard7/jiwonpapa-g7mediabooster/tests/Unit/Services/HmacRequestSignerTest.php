<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Services;

use InvalidArgumentException;
use Modules\Jiwonpapa\G7mediabooster\Services\HmacRequestSigner;
use PHPUnit\Framework\TestCase;

final class HmacRequestSignerTest extends TestCase
{
    public function testMatchesRustCanonicalContractVector(): void
    {
        $headers = (new HmacRequestSigner)->sign(
            'g7-primary',
            '0123456789abcdef0123456789abcdef',
            'POST',
            '/v1/upload-batches',
            '{"files":[]}',
            1_700_000_000,
            '0123456789abcdef0123456789abcdef',
        );

        self::assertSame('602e35a92eec4bc0a2ec6ae113f07bfc6933322fb69fe8dee416e5a67217e2a2', $headers['x-g7mb-content-sha256']);
        self::assertSame('qVBEFZK2zKdz4mjSeCznaUY-pGlNVesFKKstSumis7k', $headers['x-g7mb-signature']);
        self::assertStringNotContainsString('=', $headers['x-g7mb-signature']);
    }

    public function testRejectsControlCharactersInSignedPath(): void
    {
        $this->expectException(InvalidArgumentException::class);

        (new HmacRequestSigner)->sign(
            'g7-primary',
            '0123456789abcdef0123456789abcdef',
            'GET',
            "/v1/uploads\n/injected",
            '',
        );
    }
}
