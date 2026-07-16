<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Compatibility;

use LogicException;
use Modules\Jiwonpapa\G7mediabooster\Compatibility\Gnuboard7MediaContract;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

final class Gnuboard7MediaContractTest extends TestCase
{
    public function testCompleteVersionedContractIsAccepted(): void
    {
        Gnuboard7MediaContract::assertContractDocument($this->validContract());

        self::addToAssertionCount(1);
    }

    #[DataProvider('invalidContractProvider')]
    public function testIncompleteOrIncompatibleContractIsRejected(array $contract, string $message): void
    {
        $this->expectException(LogicException::class);
        $this->expectExceptionMessage($message);

        Gnuboard7MediaContract::assertContractDocument($contract);
    }

    /** @return iterable<string, array{array<string, mixed>, string}> */
    public static function invalidContractProvider(): iterable
    {
        $valid = self::contractFixture();

        yield 'wrong id' => [
            [...$valid, 'id' => 'untrusted.contract'],
            'G7MB_G7_CONTRACT_VERSION_UNSUPPORTED',
        ];
        yield 'future major' => [
            [...$valid, 'version' => '2.0.0'],
            'G7MB_G7_CONTRACT_VERSION_UNSUPPORTED',
        ];
        yield 'missing visibility guard' => [
            [
                ...$valid,
                'features' => array_values(array_filter(
                    $valid['features'],
                    static fn (string $feature): bool => $feature !== 'visibility_aware_delivery',
                )),
            ],
            'G7MB_G7_CONTRACT_FEATURE_MISSING:visibility_aware_delivery',
        ];
        yield 'duplicate feature' => [
            [...$valid, 'features' => [...$valid['features'], 'bounded_attachment_ids']],
            'G7MB_G7_CONTRACT_FEATURES_INVALID',
        ];
        yield 'non-string feature' => [
            [...$valid, 'features' => [...$valid['features'], ['unexpected']]],
            'G7MB_G7_CONTRACT_FEATURES_INVALID',
        ];
    }

    /** @return array<string, mixed> */
    private function validContract(): array
    {
        return self::contractFixture();
    }

    /** @return array<string, mixed> */
    private static function contractFixture(): array
    {
        return [
            'schema_version' => 1,
            'id' => 'sirsoft-board.secure-external-attachments',
            'version' => '1.0.0',
            'features' => [
                'bounded_attachment_ids',
                'owner_scoped_linking',
                'all_or_nothing_linking',
                'attachment_count_sync',
                'byte_free_delivery_authorization',
                'visibility_aware_delivery',
                'filtered_download_url',
                'filtered_preview_url',
                'video_poster_url',
                'stable_user_layout_targets',
                'stable_admin_layout_targets',
                'module_prefixed_layout_overlays',
                'all_matching_layout_targets',
            ],
        ];
    }
}
