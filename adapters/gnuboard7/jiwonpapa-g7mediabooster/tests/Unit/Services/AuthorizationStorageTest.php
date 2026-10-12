<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Unit\Services;

use LogicException;
use Modules\Jiwonpapa\G7mediabooster\Services\AuthorizationStorage;
use PHPUnit\Framework\TestCase;

final class AuthorizationStorageTest extends TestCase
{
    protected function setUp(): void
    {
        if (! interface_exists('App\\Contracts\\Extension\\StorageInterface')) {
            self::markTestSkipped('Stock host StorageInterface is required.');
        }
    }

    public function test_metadata_is_scoped_and_media_read_is_forbidden(): void
    {
        $storage = new AuthorizationStorage('approved-id');
        self::assertTrue($storage->exists('attachments', 'approved-id'));
        self::assertFalse($storage->exists('attachments', 'other-id'));
        self::assertFalse($storage->exists('settings', 'approved-id'));
        $this->expectException(LogicException::class);
        $storage->get('attachments', 'approved-id');
    }

    public function test_authorization_response_cannot_send_bytes(): void
    {
        $storage = new AuthorizationStorage('approved-id');
        self::assertNull($storage->response('attachments', 'other-id', 'file.jpg'));
        $response = $storage->response('attachments', 'approved-id', 'file.jpg');
        self::assertNotNull($response);
        $this->expectException(LogicException::class);
        $response->sendContent();
    }
}
