<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Services;

use Illuminate\Http\Client\Factory;
use Illuminate\Http\Client\Request;
use LogicException;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Exceptions\MediaBoosterUpstreamException;
use Modules\Jiwonpapa\G7mediabooster\Services\HmacRequestSigner;
use Modules\Jiwonpapa\G7mediabooster\Services\MediaBoosterClient;
use PHPUnit\Framework\TestCase;

final class MediaBoosterClientTest extends TestCase
{
    public function testSendsTheExactSignedJsonBytesWithoutFollowingRedirects(): void
    {
        $http = new Factory;
        $http->fake([
            'https://media.example.com/v1/upload-batches' => $http->response([
                'batch_id' => '018f47f0-3333-7333-8333-333333333333',
                'uploads' => [],
            ], 201),
        ]);
        $client = $this->client($http);

        $result = $client->createBatch(['files' => []]);

        self::assertSame('018f47f0-3333-7333-8333-333333333333', $result['batch_id']);
        $http->assertSent(function (Request $request): bool {
            return $request->method() === 'POST'
                && $request->url() === 'https://media.example.com/v1/upload-batches'
                && $request->body() === '{"files":[]}'
                && $request->header('x-g7mb-content-sha256')[0] === hash('sha256', '{"files":[]}')
                && $request->hasHeader('x-g7mb-signature');
        });
    }

    public function testMapsUnsafeUpstreamFailureToStableGatewayError(): void
    {
        $http = new Factory;
        $http->fake([
            '*' => $http->response('<html>proxy failure with internal details</html>', 500),
        ]);

        try {
            $this->client($http)->createBatch(['files' => []]);
            self::fail('Expected MediaBoosterUpstreamException');
        } catch (MediaBoosterUpstreamException $error) {
            self::assertSame(502, $error->httpStatus);
            self::assertSame('UPSTREAM_REQUEST_FAILED', $error->errorCode);
            self::assertStringNotContainsString('internal details', $error->getMessage());
        }
    }

    public function testPublishesAndReadsSignedSitePolicySnapshots(): void
    {
        $http = new Factory;
        $snapshot = [
            'schema_version' => 1,
            'revision' => 1,
            'issued_at' => 1_800_000_000,
            'settings_sha256' => str_repeat('a', 64),
            'watermark' => null,
        ];
        $http->fake([
            'https://media.example.com/v1/site-policy' => $http->sequence()
                ->push($snapshot, 201)
                ->push($snapshot, 200),
        ]);
        $client = $this->client($http);

        $published = $client->publishSitePolicy([
            'schema_version' => 1,
            'revision' => 1,
            'issued_at' => 1_800_000_000,
            'watermark' => null,
        ]);
        $active = $client->activeSitePolicy();

        self::assertSame(1, $published['revision']);
        self::assertSame(str_repeat('a', 64), $active['settings_sha256'] ?? null);
        $http->assertSentCount(2);
        $http->assertSent(fn (Request $request): bool => $request->url() === 'https://media.example.com/v1/site-policy'
            && $request->hasHeader('x-g7mb-signature'));
    }

    public function testTreatsMissingActiveSitePolicyAsEmptyState(): void
    {
        $http = new Factory;
        $http->fake([
            'https://media.example.com/v1/site-policy' => $http->response([
                'code' => 'SITE_POLICY_NOT_FOUND',
                'message' => 'none',
                'request_id' => 'test-request',
            ], 404),
        ]);

        self::assertNull($this->client($http)->activeSitePolicy());
    }

    public function testReadsSignedRuntimeCapabilities(): void
    {
        $http = new Factory;
        $http->fake([
            'https://media.example.com/v1/capabilities' => $http->response([
                'image_inputs' => ['avif', 'gif', 'heif', 'jpeg', 'png', 'webp'],
                'image_outputs' => ['avif', 'jpeg', 'png', 'webp'],
                'video_inputs' => ['mov', 'mp4'],
                'mp4_thumbnail' => true,
                'mp4_h264_fallback' => true,
                'native_versions' => ['vips' => 'vips-8.18.3'],
            ]),
        ]);

        $capabilities = $this->client($http)->capabilities();

        self::assertTrue($capabilities['mp4_thumbnail']);
        self::assertSame(['mov', 'mp4'], $capabilities['video_inputs']);
        $http->assertSent(fn (Request $request): bool => $request->method() === 'GET'
            && $request->url() === 'https://media.example.com/v1/capabilities'
            && $request->body() === ''
            && $request->header('x-g7mb-content-sha256')[0] === hash('sha256', '')
            && $request->hasHeader('x-g7mb-signature'));
    }

    public function testRequestsIdempotentUploadDeletionWithSignedEmptyBody(): void
    {
        $uploadId = '018f47f0-3333-7333-8333-333333333333';
        $http = new Factory;
        $http->fake([
            "https://media.example.com/v1/uploads/{$uploadId}" => $http->response(null, 202),
        ]);

        $this->client($http)->deleteUpload($uploadId);

        $http->assertSent(fn (Request $request): bool => $request->method() === 'DELETE'
            && $request->url() === "https://media.example.com/v1/uploads/{$uploadId}"
            && $request->body() === ''
            && $request->header('x-g7mb-content-sha256')[0] === hash('sha256', '')
            && $request->hasHeader('x-g7mb-signature'));
    }

    public function testRequestsPrivateDerivativeDeliveryWithSignedEmptyBody(): void
    {
        $uploadId = '018f47f0-3333-7333-8333-333333333333';
        $path = "/v1/uploads/{$uploadId}/derivatives/thumbnail/delivery";
        $http = new Factory;
        $http->fake([
            "https://media.example.com{$path}" => $http->response([
                'upload_id' => $uploadId,
                'preset_id' => 'board-v1',
                'variant' => 'thumbnail',
                'delivery_url' => 'https://private.example.com/media/file.jpg?X-Amz-Signature=redacted',
                'expires_at' => '2030-01-01T00:00:00Z',
                'content_type' => 'image/jpeg',
                'byte_len' => 512,
            ]),
        ]);

        $delivery = $this->client($http)->derivativeDelivery($uploadId, 'thumbnail');

        self::assertSame('thumbnail', $delivery['variant']);
        $http->assertSent(fn (Request $request): bool => $request->method() === 'GET'
            && $request->url() === "https://media.example.com{$path}"
            && $request->body() === ''
            && $request->header('x-g7mb-content-sha256')[0] === hash('sha256', '')
            && $request->hasHeader('x-g7mb-signature'));
    }

    public function testRejectsArbitraryDerivativeVariantBeforeNetworkIo(): void
    {
        $http = new Factory;
        $http->fake();

        $this->expectException(LogicException::class);
        try {
            $this->client($http)->derivativeDelivery(
                '018f47f0-3333-7333-8333-333333333333',
                'arbitrary',
            );
        } finally {
            $http->assertNothingSent();
        }
    }

    private function client(Factory $http): MediaBoosterClient
    {
        return new MediaBoosterClient(
            MediaBoosterConfiguration::fromArray([
                'enabled' => true,
                'control_endpoint' => 'https://media.example.com',
                'key_id' => 'g7-primary',
                'hmac_secret' => '0123456789abcdef0123456789abcdef',
                'timeout_seconds' => 15,
                'connect_timeout_seconds' => 3,
                'max_parallel_files' => 8,
                'max_parallel_parts' => 4,
                'max_part_retries' => 3,
                'status_poll_interval_ms' => 1500,
                'watermark_enabled' => false,
                'watermark_asset_upload_id' => '',
                'watermark_position' => 'bottom_right',
                'watermark_margin_px' => 24,
                'watermark_max_width_percent' => 20,
                'watermark_opacity_percent' => 80,
            ]),
            new HmacRequestSigner,
            $http,
        );
    }
}
