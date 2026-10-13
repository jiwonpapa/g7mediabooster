<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Services;

use LogicException;
use Modules\Jiwonpapa\G7mediabooster\Services\BoardUploaderLayout;
use PHPUnit\Framework\TestCase;

final class BoardUploaderLayoutTest extends TestCase
{
    public function test_native_uploader_is_replaced_without_ids_and_submit_guard_is_preserved(): void
    {
        $source = ['components' => [
            ['name' => 'FileUploader', 'props' => ['apiEndpoints' => ['upload' => '/api/modules/sirsoft-board/boards/{{route.slug}}/attachments']]],
            ['name' => 'Button', 'props' => ['disabled' => '{{_local.isSaving}}'],
                'actions' => [['handler' => 'emitEvent', 'params' => ['event' => 'upload:board_attachments']]]],
        ]];
        $actual = (new BoardUploaderLayout)->apply($source);
        self::assertSame('FileUploader', $source['components'][0]['name']);
        self::assertSame('g7mb-user-uploader-mount', $actual['components'][0]['props']['id']);
        self::assertStringContainsString('_local.isSaving', $actual['components'][1]['props']['disabled']);
        self::assertStringContainsString('g7mbUploading', $actual['components'][1]['props']['disabled']);
        self::assertSame($actual, (new BoardUploaderLayout)->apply($actual));
    }

    public function test_unrelated_uploaders_are_untouched(): void
    {
        $source = ['name' => 'FileUploader', 'props' => ['apiEndpoints' => ['upload' => '/api/other/attachments']]];
        self::assertSame($source, (new BoardUploaderLayout)->apply($source));
    }

    public function test_existing_files_and_native_management_endpoints_are_preserved(): void
    {
        foreach (['boards', 'admin/board'] as $scope) {
            $base = '/api/modules/sirsoft-board/'.$scope.'/{{route.slug}}/attachments';
            $actual = (new BoardUploaderLayout)->apply(['name' => 'FileUploader', 'props' => [
                'initialFiles' => '{{form_meta?.data?.attachments ?? []}}',
                'disabled' => '{{_computed.isReadOnly}}',
                'apiEndpoints' => ['upload' => $base, 'delete' => $base.'/:id', 'reorder' => $base.'/reorder'],
            ]]);
            $params = $actual['lifecycle']['onMount'][0]['params'];
            self::assertSame('{{form_meta?.data?.attachments ?? []}}', $params['initialFiles']);
            self::assertSame($base.'/:id', $params['deleteEndpoint']);
            self::assertSame($base.'/reorder', $params['reorderEndpoint']);
            self::assertSame('{{_computed.isReadOnly}}', $params['disabled']);
        }
    }

    public function test_save_is_guarded_by_upload_success_and_copies_ids_before_native_api(): void
    {
        $source = ['handler' => 'sequence', 'actions' => [
            ['handler' => 'emitEvent', 'params' => ['event' => 'upload:board_attachments']],
            ['handler' => 'apiCall', 'target' => '/api/modules/sirsoft-board/boards/gallery/posts',
                'if' => '{{_local.isValid}}'],
        ]];
        $actual = (new BoardUploaderLayout)->apply($source);
        self::assertSame('setState', $actual['actions'][1]['handler']);
        self::assertArrayHasKey('form.attachment_ids', $actual['actions'][1]['params']);
        self::assertStringContainsString('_eventResult?.success === true', $actual['actions'][2]['if']);
        self::assertStringContainsString('_local.isValid', $actual['actions'][2]['if']);
        self::assertSame($actual, (new BoardUploaderLayout)->apply($actual));
    }

    public function test_stock_user_and_admin_partials_are_supported_without_patches(): void
    {
        $root = getenv('G7MB_TEST_HOST_ROOT');
        if (! is_string($root) || $root === '') {
            self::markTestSkipped('Set G7MB_TEST_HOST_ROOT for real stock-source evidence.');
        }
        foreach ([
            'templates/_bundled/sirsoft-basic/layouts/partials/board/form/_post_form.json' => 'g7mb-user-uploader-mount',
            'modules/_bundled/sirsoft-board/resources/layouts/admin/partials/admin_board_post_form/_attachments.json' => 'g7mb-admin-uploader-mount',
        ] as $path => $mount) {
            $source = json_decode((string) file_get_contents($root.'/'.$path), true, flags: JSON_THROW_ON_ERROR);
            $actual = json_encode((new BoardUploaderLayout)->apply($source), JSON_THROW_ON_ERROR);
            self::assertStringContainsString($mount, $actual);
            self::assertStringNotContainsString('"name":"FileUploader"', $actual);
        }
    }

    public function test_oversized_layout_is_rejected(): void
    {
        $this->expectException(LogicException::class);
        (new BoardUploaderLayout)->apply(array_fill(0, 20_001, []));
    }
}
