<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5\Tests\Unit;

use Jiwonpapa\G7MediaBooster\Gnuboard5\Configuration;
use Jiwonpapa\G7MediaBooster\Gnuboard5\ControlClient;
use Jiwonpapa\G7MediaBooster\Gnuboard5\HmacSigner;
use Jiwonpapa\G7MediaBooster\Gnuboard5\Transport;
use Jiwonpapa\G7MediaBooster\Gnuboard5\TransportResponse;
use PHPUnit\Framework\Attributes\Test;
use PHPUnit\Framework\TestCase;

final class ControlClientTest extends TestCase
{
    #[Test]
    public function strips_network_details_and_signs_the_exact_path(): void
    {
        $transport = new class implements Transport {
            /** @var array<string, mixed> */
            public array $request = [];

            public function send(
                string $method,
                string $url,
                array $headers,
                string $body,
                int $connectTimeoutSeconds,
                int $timeoutSeconds,
            ): TransportResponse {
                $this->request = compact('method', 'url', 'headers', 'body');

                return new TransportResponse(200, json_encode([
                    'upload_id' => '018f47f0-2222-7222-8222-222222222222',
                    'state' => 'ready',
                    'deletion_pending' => false,
                    'derivatives' => [],
                ], JSON_THROW_ON_ERROR));
            }
        };
        $client = new ControlClient(new Configuration(
            true,
            'https://media.example.com',
            'g5-site-1',
            str_repeat('s', 32),
        ), new HmacSigner, $transport);

        $status = $client->status('018f47f0-2222-7222-8222-222222222222');

        self::assertSame('ready', $status['state']);
        self::assertSame('GET', $transport->request['method']);
        self::assertSame(
            'https://media.example.com/v1/uploads/018f47f0-2222-7222-8222-222222222222',
            $transport->request['url'],
        );
        self::assertArrayHasKey('x-g7mb-signature', $transport->request['headers']);
    }
}
