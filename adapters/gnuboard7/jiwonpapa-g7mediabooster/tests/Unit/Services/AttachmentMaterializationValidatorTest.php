<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Services;

use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentMaterializationValidator;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;
use UnexpectedValueException;

final class AttachmentMaterializationValidatorTest extends TestCase
{
    private const UPLOAD_ID = '018f47f0-3333-7333-8333-333333333333';

    public function testBuildsAByteFreeNativeImageAttachmentDescriptor(): void
    {
        $descriptor = (new AttachmentMaterializationValidator)->validate(
            $this->readyStatus(),
            $this->session(),
        );

        self::assertSame(0, $descriptor['board_id']);
        self::assertNull($descriptor['post_id']);
        self::assertSame('여름휴가.jpg', $descriptor['original_filename']);
        self::assertSame(self::UPLOAD_ID.'.jpg', $descriptor['stored_filename']);
        self::assertSame('g7mediabooster', $descriptor['disk']);
        self::assertSame(self::UPLOAD_ID, $descriptor['path']);
        self::assertSame('image/jpeg', $descriptor['mime_type']);
        self::assertSame(500_000, $descriptor['size']);
        self::assertSame('post_attachments', $descriptor['collection']);
        self::assertSame(1, $descriptor['order']);
        self::assertSame(self::UPLOAD_ID, $descriptor['meta']['g7mb_upload_id']);
    }

    public function testBuildsAnExactMp4MasterDescriptor(): void
    {
        $status = $this->readyStatus();
        $status['detected_content_type'] = 'video/mp4';
        $status['derivatives'][0]['content_type'] = 'video/mp4';
        $status['derivatives'][0]['byte_len'] = 10_000_000;
        $session = $this->session();
        $session['declared_kind'] = 'video';
        $session['expected_size_bytes'] = 10_000_000;
        $session['original_filename'] = 'clip.mov';

        $descriptor = (new AttachmentMaterializationValidator)->validate($status, $session);

        self::assertSame('clip.mp4', $descriptor['original_filename']);
        self::assertSame(self::UPLOAD_ID.'.mp4', $descriptor['stored_filename']);
        self::assertSame('video/mp4', $descriptor['mime_type']);
    }

    /** @return iterable<string, array{callable(array<string,mixed>,array<string,mixed>):void}> */
    public static function invalidCases(): iterable
    {
        yield 'wrong upload correlation' => [static function (array &$status): void {
            $status['upload_id'] = '018f47f0-4444-7444-8444-444444444444';
        }];
        yield 'not ready' => [static function (array &$status): void {
            $status['state'] = 'processing';
        }];
        yield 'deletion pending' => [static function (array &$status): void {
            $status['deletion_pending'] = true;
        }];
        yield 'missing thumbnail' => [static function (array &$status): void {
            array_pop($status['derivatives']);
        }];
        yield 'duplicate master' => [static function (array &$status): void {
            $status['derivatives'][1]['variant'] = 'master';
        }];
        yield 'bad thumbnail type' => [static function (array &$status): void {
            $status['derivatives'][1]['content_type'] = 'image/png';
        }];
        yield 'bad master type' => [static function (array &$status): void {
            $status['derivatives'][0]['content_type'] = 'image/png';
        }];
        yield 'preset mismatch' => [static function (array &$status): void {
            $status['derivatives'][1]['preset_id'] = 'other-v1';
        }];
        yield 'unsupported video container' => [static function (array &$status, array &$session): void {
            $session['declared_kind'] = 'video';
            $session['original_filename'] = 'clip.webm';
            $status['detected_content_type'] = 'video/webm';
            $status['derivatives'][0]['content_type'] = 'video/webm';
        }];
        yield 'path-like filename' => [static function (array &$status, array &$session): void {
            $session['original_filename'] = '../secret.jpg';
        }];
        yield 'oversized thumbnail' => [static function (array &$status): void {
            $status['derivatives'][1]['byte_len'] = 33 * 1024 * 1024;
        }];
        yield 'string encoded byte length' => [static function (array &$status): void {
            $status['derivatives'][0]['byte_len'] = '500000';
        }];
    }

    #[DataProvider('invalidCases')]
    public function testRejectsUnsafeOrIncompleteMaterialization(callable $mutate): void
    {
        $status = $this->readyStatus();
        $session = $this->session();
        $mutate($status, $session);

        $this->expectException(UnexpectedValueException::class);
        (new AttachmentMaterializationValidator)->validate($status, $session);
    }

    /** @return array<string, mixed> */
    private function session(): array
    {
        return [
            'upload_id' => self::UPLOAD_ID,
            'declared_kind' => 'image',
            'expected_size_bytes' => 1_000_000,
            'attachment_order' => 1,
            'original_filename' => '여름휴가.heic',
        ];
    }

    /** @return array<string, mixed> */
    private function readyStatus(): array
    {
        return [
            'upload_id' => self::UPLOAD_ID,
            'state' => 'ready',
            'detected_content_type' => 'image/heic',
            'error_code' => null,
            'deletion_pending' => false,
            'derivatives' => [
                [
                    'preset_id' => 'board-default-v1',
                    'variant' => 'master',
                    'url_path' => '/media/master.jpg',
                    'content_type' => 'image/jpeg',
                    'byte_len' => 500_000,
                ],
                [
                    'preset_id' => 'board-default-v1',
                    'variant' => 'thumbnail',
                    'url_path' => '/media/thumbnail.jpg',
                    'content_type' => 'image/jpeg',
                    'byte_len' => 50_000,
                ],
            ],
        ];
    }
}
