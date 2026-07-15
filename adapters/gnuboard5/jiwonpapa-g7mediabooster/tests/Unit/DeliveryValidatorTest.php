<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5\Tests\Unit;

use Jiwonpapa\G7MediaBooster\Gnuboard5\DeliveryValidator;
use PHPUnit\Framework\Attributes\Test;
use PHPUnit\Framework\TestCase;
use UnexpectedValueException;

final class DeliveryValidatorTest extends TestCase
{
    #[Test]
    public function accepts_short_lived_https_delivery(): void
    {
        $url = (new DeliveryValidator)->validate([
            'upload_id' => '018f47f0-2222-7222-8222-222222222222',
            'variant' => 'thumbnail',
            'content_type' => 'image/jpeg',
            'byte_len' => 512,
            'expires_at' => gmdate(DATE_RFC3339, time() + 300),
            'delivery_url' => 'https://objects.example.com/media/thumb?signature=redacted',
        ], '018f47f0-2222-7222-8222-222222222222', 'thumbnail');

        self::assertStringStartsWith('https://objects.example.com/', $url);
    }

    #[Test]
    public function rejects_header_injection_in_delivery_url(): void
    {
        $this->expectException(UnexpectedValueException::class);

        (new DeliveryValidator)->validate([
            'upload_id' => '018f47f0-2222-7222-8222-222222222222',
            'variant' => 'master',
            'content_type' => 'image/jpeg',
            'byte_len' => 512,
            'expires_at' => gmdate(DATE_RFC3339, time() + 300),
            'delivery_url' => "https://objects.example.com/file\r\nX-Evil: 1",
        ], '018f47f0-2222-7222-8222-222222222222', 'master');
    }
}
