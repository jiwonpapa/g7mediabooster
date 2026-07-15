<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit;

use Illuminate\Console\Command;
use Modules\Jiwonpapa\G7mediabooster\Console\Commands\ReconcileAttachmentRetentionCommand;
use PHPUnit\Framework\TestCase;

final class BridgeContractSourceTest extends TestCase
{
    public function testBridgeRoutesAndStorageStayFailClosed(): void
    {
        $root = dirname(__DIR__, 2);
        $routes = (string) file_get_contents($root.'/src/routes/api.php');
        $store = (string) file_get_contents($root.'/src/Services/UploadSessionStore.php');
        $bridge = (string) file_get_contents($root.'/src/Services/AttachmentBridgeService.php');
        $delivery = (string) file_get_contents($root.'/src/Http/Controllers/User/AttachmentDeliveryController.php');
        $upload = (string) file_get_contents($root.'/src/Http/Controllers/User/UploadController.php');
        $batchRequest = (string) file_get_contents($root.'/src/Http/Requests/CreateUploadBatchRequest.php');
        $module = (string) file_get_contents($root.'/module.php');
        $migration = (string) file_get_contents($root.'/database/migrations/2026_07_15_000002_add_attachment_bridge_to_g7mb_upload_sessions.php');
        $retentionMigration = (string) file_get_contents($root.'/database/migrations/2026_07_15_000003_add_retention_queue_to_g7mb_upload_sessions.php');
        $retention = (string) file_get_contents($root.'/src/Services/AttachmentRetentionService.php');
        $lifecycle = (string) file_get_contents($root.'/src/Listeners/AttachmentLifecycleListener.php');
        $command = (string) file_get_contents($root.'/src/Console/Commands/ReconcileAttachmentRetentionCommand.php');
        $catalog = (string) file_get_contents($root.'/src/Services/WatermarkAssetCatalog.php');
        $settingsController = (string) file_get_contents($root.'/src/Http/Controllers/Admin/SettingsController.php');
        $settingsLayout = $this->decodeExtension($root.'/resources/layouts/admin/admin_media_booster_settings.json');

        self::assertStringContainsString("Route::post('{uploadId}/attachment'", $routes);
        self::assertStringContainsString("'optional.sanctum'", $routes);
        self::assertStringContainsString('attachments.download', $routes);
        self::assertStringContainsString('lockForUpdate()', $store);
        self::assertStringContainsString("->whereNull('attachment_id')", $store);
        self::assertStringContainsString('assertSecureUpstreamContract()', $bridge);
        self::assertStringContainsString('findPostForAttachmentDelivery', $bridge);
        self::assertStringContainsString('visibility-aware attachment delivery is unavailable', $bridge);
        self::assertStringContainsString("->where('board_id', 0)", $bridge);
        self::assertStringContainsString('isMaterializedAs(', $delivery);
        self::assertStringContainsString('authorizeDelivery(', $delivery);
        self::assertStringContainsString("unset(\$file['original_filename'])", $upload);
        self::assertStringNotContainsString('video/webm', $batchRequest);
        self::assertStringContainsString('video/quicktime', $batchRequest);
        self::assertStringContainsString('AttachmentUrlListener::class', $module);
        self::assertStringContainsString('nullOnDelete()', $migration);
        self::assertStringContainsString('retention_request_started_at', $retentionMigration);
        self::assertStringContainsString('lockForUpdate()', $retention);
        self::assertStringContainsString('G7_MEDIA_RETENTION_ALREADY_STARTED', $retention);
        self::assertStringContainsString('sirsoft-board.post.before_restore', $lifecycle);
        self::assertStringContainsString('keepInFlight:', $command);
        self::assertStringContainsString('g7mediabooster:reconcile-attachment-retention', $module);
        self::assertStringContainsString("Route::get('watermark-assets'", $routes);
        self::assertStringContainsString("->where('sessions.user_id', \$userId)", $catalog);
        self::assertStringContainsString("->where('sessions.state', 'ready')", $catalog);
        self::assertStringContainsString("->where('attachments.collection', 'post_attachments')", $catalog);
        self::assertStringContainsString("->where('attachments.created_by', \$userId)", $catalog);
        self::assertStringContainsString('isSelectableForUser($adminId, $watermarkAssetId)', $settingsController);
        self::assertStringNotContainsString('Ready 상태 이미지 UUID', json_encode($settingsLayout, JSON_THROW_ON_ERROR));
        self::assertStringContainsString(
            'jiwonpapa-g7mediabooster.mountWatermarkPicker',
            json_encode($settingsLayout, JSON_THROW_ON_ERROR),
        );
        self::assertTrue(is_subclass_of(ReconcileAttachmentRetentionCommand::class, Command::class));
    }

    public function testBoardFormExtensionsMountUploaderAndBlockSubmitWhileRunning(): void
    {
        $root = dirname(__DIR__, 2);
        $user = $this->decodeExtension($root.'/resources/extensions/user-board-media-uploader.json');
        $admin = $this->decodeExtension($root.'/resources/extensions/admin-board-media-uploader.json');

        self::assertSame('board/form', $user['target_layout']);
        self::assertSame('sirsoft-board.admin_board_post_form', $admin['target_layout']);
        $this->assertUploaderInjection($user, 'board_native_file_uploader', 'board_post_submit');
        $this->assertUploaderInjection($admin, 'admin_board_native_file_uploader', 'footer_save_button');
    }

    /** @return array<string, mixed> */
    private function decodeExtension(string $path): array
    {
        $decoded = json_decode((string) file_get_contents($path), true, flags: JSON_THROW_ON_ERROR);
        self::assertIsArray($decoded);

        return $decoded;
    }

    /** @param array<string, mixed> $extension */
    private function assertUploaderInjection(array $extension, string $uploaderTarget, string $submitTarget): void
    {
        self::assertIsArray($extension['injections'] ?? null);
        self::assertCount(2, $extension['injections']);
        $uploader = $extension['injections'][0];
        $submit = $extension['injections'][1];
        self::assertIsArray($uploader);
        self::assertIsArray($submit);
        self::assertSame($uploaderTarget, $uploader['target_id'] ?? null);
        self::assertSame('replace', $uploader['position'] ?? null);
        self::assertSame(
            'jiwonpapa-g7mediabooster.mountUploader',
            $uploader['components'][0]['lifecycle']['onMount'][0]['handler'] ?? null,
        );
        self::assertSame($submitTarget, $submit['target_id'] ?? null);
        self::assertSame('inject_props', $submit['position'] ?? null);
        self::assertStringContainsString('g7mbUploading', (string) ($submit['props']['disabled'] ?? ''));
    }
}
