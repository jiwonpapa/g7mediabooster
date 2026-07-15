<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5\Tests\Unit;

use Jiwonpapa\G7MediaBooster\Gnuboard5\ReadyAssetValidator;
use PHPUnit\Framework\Attributes\Test;
use PHPUnit\Framework\TestCase;
use UnexpectedValueException;

final class ReadyAssetValidatorTest extends TestCase
{
    #[Test]
    public function valid_ready_image_becomes_a_remote_g5_file_record(): void
    {
        $materialized = (new ReadyAssetValidator)->validate($this->readyStatus(), $this->session());

        self::assertSame('사진.jpg', $materialized['original_filename']);
        self::assertSame('image/jpeg', $materialized['mime_type']);
        self::assertSame(2, $materialized['image_type']);
    }

    #[Test]
    public function missing_thumbnail_is_rejected(): void
    {
        $status = $this->readyStatus();
        array_pop($status['derivatives']);
        $this->expectException(UnexpectedValueException::class);

        (new ReadyAssetValidator)->validate($status, $this->session());
    }

    #[Test]
    public function valid_ready_quicktime_becomes_a_remote_g5_video_record(): void
    {
        $status = $this->readyStatus();
        $status['detected_content_type'] = 'video/quicktime';
        $status['derivatives'][0]['content_type'] = 'video/quicktime';
        $status['derivatives'][0]['byte_len'] = 4096;
        $session = $this->session();
        $session['declared_kind'] = 'video';
        $session['original_filename'] = '영상.mp4';

        $materialized = (new ReadyAssetValidator)->validate($status, $session);

        self::assertSame('영상.mov', $materialized['original_filename']);
        self::assertStringEndsWith('.mov', $materialized['stored_filename']);
        self::assertSame('video/quicktime', $materialized['mime_type']);
        self::assertSame(0, $materialized['image_type']);
    }

    /** @return array<string, mixed> */
    private function session(): array
    {
        return [
            'upload_id' => '018f47f0-2222-7222-8222-222222222222',
            'original_filename' => '사진.avif',
            'declared_kind' => 'image',
            'expected_size_bytes' => 4096,
            'attachment_order' => 1,
        ];
    }

    /** @return array<string, mixed> */
    private function readyStatus(): array
    {
        return [
            'upload_id' => '018f47f0-2222-7222-8222-222222222222',
            'state' => 'ready',
            'deletion_pending' => false,
            'detected_content_type' => 'image/avif',
            'derivatives' => [
                [
                    'variant' => 'master',
                    'preset_id' => 'default-v1',
                    'url_path' => '/media/master.jpg',
                    'content_type' => 'image/jpeg',
                    'byte_len' => 3072,
                ],
                [
                    'variant' => 'thumbnail',
                    'preset_id' => 'default-v1',
                    'url_path' => '/media/thumb.jpg',
                    'content_type' => 'image/jpeg',
                    'byte_len' => 512,
                ],
            ],
        ];
    }
}
