<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Host;

require_once dirname(__DIR__, 3).'/sirsoft-board/tests/BoardTestCase.php';

use Illuminate\Support\Carbon;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Schema;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Http\Controllers\Admin\SettingsController;
use Modules\Jiwonpapa\G7mediabooster\Http\Requests\UpdateSettingsRequest;
use Modules\Jiwonpapa\G7mediabooster\Listeners\AttachmentLifecycleListener;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentBridgeService;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentRetentionDecision;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentRetentionService;
use Modules\Jiwonpapa\G7mediabooster\Services\WatermarkAssetCatalog;
use Modules\Sirsoft\Board\Models\Post;
use Modules\Sirsoft\Board\Tests\BoardTestCase;
use PHPUnit\Framework\Attributes\Test;
use RuntimeException;

final class AttachmentRetentionHostTest extends BoardTestCase
{
    private const UPLOAD_ID = '018f47f0-4444-7444-8444-444444444444';

    private AttachmentRetentionService $retention;

    private AttachmentLifecycleListener $listener;

    protected function getTestBoardSlug(): string
    {
        return 'g7mb-retention-host';
    }

    protected function runModuleMigrationIfNeeded(): void
    {
        parent::runModuleMigrationIfNeeded();
        if (Schema::hasTable('g7mb_upload_sessions')) {
            return;
        }

        $this->artisan('migrate', [
            '--path' => dirname(__DIR__, 2).'/database/migrations',
            '--realpath' => true,
        ]);
    }

    protected function setUp(): void
    {
        parent::setUp();

        self::assertTrue(
            Schema::hasTable('g7mb_upload_sessions'),
            'G7MediaBooster migrations must be loaded by the Gnuboard7 host test.',
        );
        DB::table('g7mb_upload_sessions')->where('board_slug', $this->board->slug)->delete();

        $this->retention = new AttachmentRetentionService(new AttachmentRetentionDecision);
        $configuration = MediaBoosterConfiguration::fromArray(['attachment_retention_days' => 7]);
        $this->listener = new AttachmentLifecycleListener($this->retention, $configuration);
    }

    #[Test]
    public function secure_bridge_accepts_visibility_aware_upstream_contract(): void
    {
        AttachmentBridgeService::assertSecureUpstreamContract();

        $this->addToAssertionCount(1);
    }

    #[Test]
    public function watermark_catalog_only_exposes_current_admins_valid_ready_sources(): void
    {
        $admin = $this->createAdminUser();
        $other = $this->createUser();
        $validId = '018f47f0-1001-7001-8001-000000000001';
        $this->catalogCandidate($validId, (int) $admin->getKey(), 'image/png');
        $this->catalogCandidate('018f47f0-1002-7002-8002-000000000002', (int) $other->getKey(), 'image/webp');
        $this->catalogCandidate('018f47f0-1003-7003-8003-000000000003', (int) $admin->getKey(), 'image/avif');
        $this->catalogCandidate(
            '018f47f0-1004-7004-8004-000000000004',
            (int) $admin->getKey(),
            'image/jpeg',
            ['expected_size_bytes' => (16 * 1024 * 1024) + 1],
        );
        $this->catalogCandidate(
            '018f47f0-1005-7005-8005-000000000005',
            (int) $admin->getKey(),
            'image/jpeg',
            attachmentOverrides: ['deleted_at' => now()],
        );
        $this->catalogCandidate(
            '018f47f0-1006-7006-8006-000000000006',
            (int) $admin->getKey(),
            'image/jpeg',
            ['declared_kind' => 'video'],
        );
        $this->catalogCandidate(
            '018f47f0-1007-7007-8007-000000000007',
            (int) $admin->getKey(),
            'image/jpeg',
            attachmentOverrides: ['collection' => 'untrusted'],
        );
        $this->catalogCandidate(
            '018f47f0-1008-7008-8008-000000000008',
            (int) $admin->getKey(),
            'image/jpeg',
            attachmentOverrides: ['created_by' => (int) $other->getKey()],
        );

        $catalog = new WatermarkAssetCatalog;
        $assets = $catalog->forUser((int) $admin->getKey());

        self::assertCount(1, $assets);
        self::assertSame($validId, $assets[0]['upload_id']);
        self::assertSame('image/png', $assets[0]['detected_content_type']);
        self::assertSame('watermark-0001.png', $assets[0]['filename']);
        self::assertTrue($catalog->isSelectableForUser((int) $admin->getKey(), $validId));
        self::assertFalse($catalog->isSelectableForUser(
            (int) $admin->getKey(),
            '018f47f0-1002-7002-8002-000000000002',
        ));
        self::assertFalse($catalog->isSelectableForUser((int) $admin->getKey(), 'not-a-uuid'));
    }

    #[Test]
    public function settings_api_rejects_a_uuid_outside_the_current_admin_catalog(): void
    {
        $admin = $this->createAdminUser();
        $this->actingAs($admin);
        $request = UpdateSettingsRequest::create('/admin/settings', 'PUT', [
            'enabled' => false,
            'control_endpoint' => 'http://127.0.0.1:8088',
            'key_id' => 'g7-primary',
            'hmac_secret' => '',
            'timeout_seconds' => 15,
            'connect_timeout_seconds' => 3,
            'max_parallel_files' => 8,
            'max_parallel_parts' => 4,
            'max_part_retries' => 3,
            'status_poll_interval_ms' => 1500,
            'attachment_retention_days' => 30,
            'watermark_enabled' => false,
            'watermark_asset_upload_id' => '018f47f0-1999-7999-8999-000000000999',
            'watermark_position' => 'bottom_right',
            'watermark_margin_px' => 24,
            'watermark_max_width_percent' => 20,
            'watermark_opacity_percent' => 80,
        ]);
        $request->setContainer($this->app);
        $request->setRedirector($this->app->make('redirect'));
        $request->validateResolved();

        $response = $this->app->make(SettingsController::class)->update($request);

        self::assertSame(422, $response->getStatusCode());
        self::assertStringContainsString('watermark_asset_upload_id', (string) $response->getContent());
    }

    #[Test]
    public function post_delete_schedules_retention_and_restore_hooks_cancel_it(): void
    {
        [$post, $attachmentId] = $this->materializedAttachment();

        $this->listener->postDeleted($post, $this->board->slug);
        $scheduled = $this->sessionRow();

        self::assertSame('post_delete', $scheduled->retention_reason);
        self::assertTrue(
            Carbon::parse($scheduled->retention_delete_after)->between(
                now()->addDays(7)->subMinute(),
                now()->addDays(7)->addMinute(),
            ),
        );

        $this->listener->postRestoring($post, null, $this->board->slug);
        self::assertNull($this->sessionRow()->retention_delete_after);

        $this->listener->postDeleted($post, $this->board->slug);
        DB::table('board_attachments')->where('id', $attachmentId)->update(['deleted_at' => null]);
        $this->listener->postRestored($post, $this->board->slug);
        self::assertNull($this->sessionRow()->retention_delete_after);
    }

    #[Test]
    public function restore_is_blocked_after_remote_delete_request_starts(): void
    {
        [$post] = $this->materializedAttachment();
        $this->listener->postDeleted($post, $this->board->slug);
        DB::table('g7mb_upload_sessions')
            ->where('upload_id', self::UPLOAD_ID)
            ->update([
                'retention_request_started_at' => now(),
                'retention_lease_until' => now()->addMinutes(10),
            ]);

        try {
            $this->listener->postRestoring($post, null, $this->board->slug);
            self::fail('restore must fail closed after remote deletion starts');
        } catch (RuntimeException $error) {
            self::assertSame('G7_MEDIA_RETENTION_ALREADY_STARTED', $error->getMessage());
        }

        self::assertNotNull($this->sessionRow()->retention_request_started_at);
    }

    #[Test]
    public function due_retention_is_leased_rechecked_and_completed(): void
    {
        [$post] = $this->materializedAttachment();
        $this->listener->postDeleted($post, $this->board->slug);
        DB::table('g7mb_upload_sessions')
            ->where('upload_id', self::UPLOAD_ID)
            ->update(['retention_delete_after' => now()->subMinute()]);

        $claims = $this->retention->claimDue(1);

        self::assertCount(1, $claims);
        self::assertSame(1, (int) $this->sessionRow()->retention_attempts);
        self::assertNotNull($this->sessionRow()->retention_lease_until);
        self::assertSame(AttachmentRetentionDecision::DELETE, $this->retention->beginClaim($claims[0]));
        self::assertSame('deletion_requesting', $this->sessionRow()->state);
        self::assertNotNull($this->sessionRow()->retention_request_started_at);

        $this->retention->completeClaim(self::UPLOAD_ID);

        self::assertSame('deletion_pending', $this->sessionRow()->state);
        self::assertNotNull($this->sessionRow()->deletion_requested_at);
        self::assertNull($this->sessionRow()->retention_delete_after);
        self::assertNull($this->sessionRow()->retention_lease_until);
    }

    #[Test]
    public function restore_race_cancels_claim_before_remote_delete(): void
    {
        [$post, $attachmentId] = $this->materializedAttachment();
        $this->listener->postDeleted($post, $this->board->slug);
        DB::table('g7mb_upload_sessions')
            ->where('upload_id', self::UPLOAD_ID)
            ->update(['retention_delete_after' => now()->subMinute()]);
        $claims = $this->retention->claimDue(1);
        DB::table('board_attachments')->where('id', $attachmentId)->update(['deleted_at' => null]);

        self::assertSame(AttachmentRetentionDecision::CANCEL, $this->retention->beginClaim($claims[0]));

        $session = $this->sessionRow();
        self::assertSame('ready', $session->state);
        self::assertNull($session->retention_delete_after);
        self::assertNull($session->retention_lease_until);
        self::assertNull($session->retention_request_started_at);
    }

    /** @return array{Post, int} */
    private function materializedAttachment(): array
    {
        $postId = $this->createTestPost();
        $attachmentId = DB::table('board_attachments')->insertGetId([
            'board_id' => $this->board->id,
            'post_id' => $postId,
            'hash' => 'g7mbhost0001',
            'original_filename' => 'host.jpg',
            'stored_filename' => self::UPLOAD_ID.'.jpg',
            'disk' => 'g7mediabooster',
            'path' => self::UPLOAD_ID,
            'mime_type' => 'image/jpeg',
            'size' => 1024,
            'collection' => 'post_attachments',
            'order' => 1,
            'trigger_type' => 'cascade',
            'created_at' => now(),
            'updated_at' => now(),
            'deleted_at' => now(),
        ]);
        DB::table('g7mb_upload_sessions')->insert([
            'upload_id' => self::UPLOAD_ID,
            'batch_id' => '018f47f0-5555-7555-8555-555555555555',
            'user_id' => null,
            'board_slug' => $this->board->slug,
            'client_ref' => 'host-retention-1',
            'original_filename' => 'host.jpg',
            'declared_kind' => 'image',
            'content_type_hint' => 'image/jpeg',
            'attachment_order' => 1,
            'transfer_method' => 'single_put',
            'expected_size_bytes' => 1024,
            'state' => 'ready',
            'attachment_id' => $attachmentId,
            'materialized_at' => now(),
            'ownership_expires_at' => now()->addHour(),
            'created_at' => now(),
            'updated_at' => now(),
        ]);

        return [Post::query()->findOrFail($postId), $attachmentId];
    }

    /**
     * @param array<string, mixed> $sessionOverrides
     * @param array<string, mixed> $attachmentOverrides
     */
    private function catalogCandidate(
        string $uploadId,
        int $userId,
        string $detectedType,
        array $sessionOverrides = [],
        array $attachmentOverrides = [],
    ): void {
        $suffix = substr($uploadId, -4);
        $attachmentId = DB::table('board_attachments')->insertGetId(array_replace([
            'board_id' => $this->board->id,
            'post_id' => null,
            'hash' => 'catasset'.$suffix,
            'original_filename' => 'watermark-'.$suffix.'.jpg',
            'stored_filename' => $uploadId.'.jpg',
            'disk' => 'g7mediabooster',
            'path' => $uploadId,
            'mime_type' => 'image/jpeg',
            'size' => 2048,
            'collection' => 'post_attachments',
            'order' => 1,
            'meta' => json_encode([
                'g7mb_upload_id' => $uploadId,
                'g7mb_preset_id' => 'image-default',
                'g7mb_detected_content_type' => $detectedType,
                'g7mb_thumbnail_content_type' => 'image/jpeg',
                'g7mb_thumbnail_size' => 1024,
            ], JSON_THROW_ON_ERROR),
            'created_by' => $userId,
            'created_at' => now(),
            'updated_at' => now(),
            'deleted_at' => null,
        ], $attachmentOverrides));
        DB::table('g7mb_upload_sessions')->insert(array_replace([
            'upload_id' => $uploadId,
            'batch_id' => '018f47f0-2000-7000-8000-'.substr($uploadId, -12),
            'user_id' => $userId,
            'board_slug' => $this->board->slug,
            'client_ref' => 'watermark-'.$suffix,
            'original_filename' => 'watermark-'.$suffix.'.png',
            'declared_kind' => 'image',
            'content_type_hint' => $detectedType,
            'attachment_order' => 1,
            'transfer_method' => 'single_put',
            'expected_size_bytes' => 4096,
            'state' => 'ready',
            'attachment_id' => $attachmentId,
            'materialized_at' => now(),
            'ownership_expires_at' => now()->addDays(7),
            'created_at' => now(),
            'updated_at' => now(),
        ], $sessionOverrides));
    }

    private function sessionRow(): object
    {
        return DB::table('g7mb_upload_sessions')->where('upload_id', self::UPLOAD_ID)->firstOrFail();
    }
}
