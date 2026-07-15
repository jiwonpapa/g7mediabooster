<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Services;

use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentRetentionDecision;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

final class AttachmentRetentionDecisionTest extends TestCase
{
    /** @return iterable<string, array{array<string, mixed>|null, string}> */
    public static function cases(): iterable
    {
        yield 'hard deleted native row' => [null, AttachmentRetentionDecision::DELETE];
        yield 'restored native row' => [[
            'disk' => 'g7mediabooster',
            'collection' => 'post_attachments',
            'path' => '018f47f0-3333-7333-8333-333333333333',
            'deleted_at' => null,
        ], AttachmentRetentionDecision::CANCEL];
        yield 'soft deleted native row' => [[
            'disk' => 'g7mediabooster',
            'collection' => 'post_attachments',
            'path' => '018f47f0-3333-7333-8333-333333333333',
            'deleted_at' => '2026-07-15 12:00:00',
        ], AttachmentRetentionDecision::DELETE];
        yield 'mismatched storage mapping' => [[
            'disk' => 'local',
            'collection' => 'post_attachments',
            'path' => '018f47f0-3333-7333-8333-333333333333',
            'deleted_at' => '2026-07-15 12:00:00',
        ], AttachmentRetentionDecision::BLOCK];
    }

    /** @param array<string, mixed>|null $attachment */
    #[DataProvider('cases')]
    public function testMakesFailClosedRetentionDecision(?array $attachment, string $expected): void
    {
        $decision = new AttachmentRetentionDecision;

        self::assertSame(
            $expected,
            $decision->evaluate($attachment, '018f47f0-3333-7333-8333-333333333333'),
        );
    }
}
