<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit;

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

        self::assertStringContainsString("Route::post('{uploadId}/attachment'", $routes);
        self::assertStringContainsString("'optional.sanctum'", $routes);
        self::assertStringContainsString('attachments.download', $routes);
        self::assertStringContainsString('lockForUpdate()', $store);
        self::assertStringContainsString("->whereNull('attachment_id')", $store);
        self::assertStringContainsString('assertSecureUpstreamContract()', $bridge);
        self::assertStringContainsString("->where('board_id', 0)", $bridge);
        self::assertStringContainsString('isMaterializedAs(', $delivery);
        self::assertStringContainsString('authorizeDelivery(', $delivery);
        self::assertStringContainsString("unset(\$file['original_filename'])", $upload);
        self::assertStringNotContainsString('video/webm', $batchRequest);
        self::assertStringNotContainsString('video/quicktime', $batchRequest);
        self::assertStringContainsString('AttachmentUrlListener::class', $module);
        self::assertStringContainsString('nullOnDelete()', $migration);
    }
}
