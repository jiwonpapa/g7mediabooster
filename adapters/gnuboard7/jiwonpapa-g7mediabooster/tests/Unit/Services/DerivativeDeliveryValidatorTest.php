<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Services;

use Modules\Jiwonpapa\G7mediabooster\Services\DerivativeDeliveryValidator;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;
use UnexpectedValueException;

final class DerivativeDeliveryValidatorTest extends TestCase
{
    private const UPLOAD_ID = '018f47f0-3333-7333-8333-333333333333';

    public function testAcceptsMatchingFutureHttpsDelivery(): void
    {
        $url = (new DerivativeDeliveryValidator)->validate(
            self::delivery('https://private.example.com/media/file.jpg?X-Amz-Signature=redacted'),
            self::UPLOAD_ID,
            'thumbnail',
        );

        self::assertStringStartsWith('https://private.example.com/', $url);
    }

    /** @param array<string, mixed> $delivery */
    #[DataProvider('invalidDeliveries')]
    public function testRejectsUntrustedDeliveryResponses(array $delivery, string $variant): void
    {
        $this->expectException(UnexpectedValueException::class);

        (new DerivativeDeliveryValidator)->validate($delivery, self::UPLOAD_ID, $variant);
    }

    /** @return iterable<string, array{array<string, mixed>, string}> */
    public static function invalidDeliveries(): iterable
    {
        $valid = self::delivery('https://private.example.com/file?signature=x');

        yield 'plain HTTP' => [array_replace($valid, ['delivery_url' => 'http://example.com/file']), 'thumbnail'];
        yield 'credential URL' => [array_replace($valid, ['delivery_url' => 'https://user:pass@example.com/file']), 'thumbnail'];
        yield 'fragment URL' => [array_replace($valid, ['delivery_url' => 'https://example.com/file#token']), 'thumbnail'];
        yield 'wrong upload' => [array_replace($valid, ['upload_id' => '018f47f0-4444-7444-8444-444444444444']), 'thumbnail'];
        yield 'wrong variant' => [$valid, 'master'];
        yield 'expired' => [array_replace($valid, ['expires_at' => '2020-01-01T00:00:00Z']), 'thumbnail'];
        yield 'natural language time' => [array_replace($valid, ['expires_at' => 'tomorrow']), 'thumbnail'];
        yield 'control byte' => [array_replace($valid, ['delivery_url' => "https://example.com/file\r\nX-Test: x"]), 'thumbnail'];
    }

    /** @return array<string, mixed> */
    private static function delivery(string $url): array
    {
        return [
            'upload_id' => self::UPLOAD_ID,
            'preset_id' => 'board-v1',
            'variant' => 'thumbnail',
            'delivery_url' => $url,
            'expires_at' => gmdate('Y-m-d\TH:i:s\Z', time() + 300),
            'content_type' => 'image/jpeg',
            'byte_len' => 512,
        ];
    }
}
