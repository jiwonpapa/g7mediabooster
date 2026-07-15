<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5\Tests\Unit;

use Jiwonpapa\G7MediaBooster\Gnuboard5\BatchValidator;
use PHPUnit\Framework\Attributes\Test;
use PHPUnit\Framework\TestCase;
use UnexpectedValueException;

final class BatchValidatorTest extends TestCase
{
    #[Test]
    public function request_and_single_put_response_are_strictly_validated(): void
    {
        $validator = new BatchValidator;
        $files = $validator->validateRequest([[
            'client_ref' => 'file_1',
            'original_filename' => '사진.jpg',
            'declared_kind' => 'image',
            'content_length' => 1024,
            'content_type_hint' => 'image/jpeg',
        ]], 10, 2048);
        $response = $validator->validateResponse([
            'batch_id' => '018f47f0-1111-7111-8111-111111111111',
            'uploads' => [[
                'client_ref' => 'file_1',
                'upload_id' => '018f47f0-2222-7222-8222-222222222222',
                'method' => 'single_put',
                'required_headers' => ['content-type' => 'image/jpeg'],
                'expires_at' => '2099-01-01T00:00:00Z',
                'part_size_bytes' => null,
                'upload_url' => 'https://objects.example.com/raw/file?signature=redacted',
            ]],
        ], $files);

        self::assertSame('사진.jpg', $files[0]['original_filename']);
        self::assertSame('018f47f0-2222-7222-8222-222222222222', $response['uploads'][0]['upload_id']);
    }

    #[Test]
    public function request_rejects_duplicate_client_references(): void
    {
        $this->expectException(UnexpectedValueException::class);

        (new BatchValidator)->validateRequest([
            [
                'client_ref' => 'same',
                'original_filename' => 'a.jpg',
                'declared_kind' => 'image',
                'content_length' => 1,
                'content_type_hint' => 'image/jpeg',
            ],
            [
                'client_ref' => 'same',
                'original_filename' => 'b.jpg',
                'declared_kind' => 'image',
                'content_length' => 1,
                'content_type_hint' => 'image/jpeg',
            ],
        ], 10, 10);
    }

    #[Test]
    public function request_accepts_release_supported_quicktime_video(): void
    {
        $files = (new BatchValidator)->validateRequest([[
            'client_ref' => 'mov_1',
            'original_filename' => 'clip.mov',
            'declared_kind' => 'video',
            'content_length' => 1024,
            'content_type_hint' => 'video/quicktime',
        ]], 10, 2048);

        self::assertSame('video/quicktime', $files[0]['content_type_hint']);
    }

    #[Test]
    public function multipart_response_accepts_the_empty_header_object_from_the_control_api(): void
    {
        $validator = new BatchValidator;
        $files = $validator->validateRequest([[
            'client_ref' => 'large_1',
            'original_filename' => 'large.jpg',
            'declared_kind' => 'image',
            'content_length' => 9_000_000,
            'content_type_hint' => 'image/jpeg',
        ]], 10, 10_000_000);

        $response = $validator->validateResponse([
            'batch_id' => '018f47f0-1111-7111-8111-111111111111',
            'uploads' => [[
                'client_ref' => 'large_1',
                'upload_id' => '018f47f0-2222-7222-8222-222222222222',
                'method' => 'multipart',
                'required_headers' => [],
                'expires_at' => '2099-01-01T00:00:00Z',
                'part_size_bytes' => 5 * 1024 * 1024,
                'upload_url' => null,
            ]],
        ], $files);

        self::assertSame('multipart', $response['uploads'][0]['method']);
    }

    #[Test]
    public function presigned_part_matches_the_actual_control_api_contract(): void
    {
        $response = (new BatchValidator)->validatePresignedPart([
            'part_number' => 1,
            'upload_url' => 'http://127.0.0.1:9000/raw/file?signature=redacted',
            'required_headers' => ['content-length' => '5242880'],
            'expires_at' => '2099-01-01T00:00:00Z',
        ], '018f47f0-2222-7222-8222-222222222222', 1);

        self::assertSame(1, $response['part_number']);
        self::assertArrayNotHasKey('upload_id', $response);
    }
}
