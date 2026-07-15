<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Services;

use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentUrlResolver;
use PHPUnit\Framework\TestCase;
use stdClass;

final class AttachmentUrlResolverTest extends TestCase
{
    public function testResolvesOnlyExactMediaBoosterAttachmentContracts(): void
    {
        $attachment = (object) [
            'disk' => 'g7mediabooster',
            'path' => '018f47f0-3333-7333-8333-333333333333',
            'hash' => 'AbC123xYz789',
        ];
        $resolver = new AttachmentUrlResolver;

        self::assertSame(
            '/api/modules/jiwonpapa-g7mediabooster/boards/notice/attachments/AbC123xYz789/master',
            $resolver->resolve(
                '/api/modules/sirsoft-board/boards/notice/attachment/AbC123xYz789',
                $attachment,
                'master',
            ),
        );
        self::assertSame(
            '/api/modules/jiwonpapa-g7mediabooster/boards/notice/attachments/AbC123xYz789/thumbnail',
            $resolver->resolve(null, $attachment, 'thumbnail', 'notice'),
        );
    }

    public function testPreservesNativeUrlForNonMatchingRows(): void
    {
        $native = '/api/modules/sirsoft-board/boards/notice/attachment/AbC123xYz789';
        $attachment = new stdClass;
        $attachment->disk = 'local';
        $attachment->path = 'notice/file.jpg';
        $attachment->hash = 'AbC123xYz789';

        self::assertSame($native, (new AttachmentUrlResolver)->resolve($native, $attachment, 'master'));
    }

    public function testFailsClosedWithoutAValidatedBoardSlug(): void
    {
        $attachment = (object) [
            'disk' => 'g7mediabooster',
            'path' => '018f47f0-3333-7333-8333-333333333333',
            'hash' => 'AbC123xYz789',
        ];

        self::assertNull((new AttachmentUrlResolver)->resolve(null, $attachment, 'thumbnail'));
    }
}
