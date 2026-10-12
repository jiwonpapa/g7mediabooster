<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Tests\Host;

require_once dirname(__DIR__, 3).'/sirsoft-board/tests/BoardTestCase.php';

use Illuminate\Http\JsonResponse;
use Illuminate\Http\Request;
use Illuminate\Pipeline\Pipeline;
use Illuminate\Routing\Route;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Http;
use Illuminate\Support\Facades\Schema;
use Illuminate\Validation\ValidationException;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Http\Middleware\AttachmentLinkMiddleware;
use Modules\Jiwonpapa\G7mediabooster\Http\Middleware\NativeDeliveryMiddleware;
use Modules\Jiwonpapa\G7mediabooster\Listeners\BridgeRemovalListener;
use Modules\Jiwonpapa\G7mediabooster\Services\BridgeRemovalGuard;
use Modules\Jiwonpapa\G7mediabooster\Services\NativeAttachmentAuthorizer;
use Modules\Jiwonpapa\G7mediabooster\Services\UploadSessionStore;
use Modules\Sirsoft\Board\Models\Attachment;
use Modules\Sirsoft\Board\Models\Post;
use Modules\Sirsoft\Board\Repositories\Contracts\AttachmentRepositoryInterface;
use Modules\Sirsoft\Board\Services\PostService;
use Modules\Sirsoft\Board\Tests\BoardTestCase;
use Symfony\Component\HttpKernel\Exception\AccessDeniedHttpException;

final class CoreFreeBridgeHostTest extends BoardTestCase
{
    public function test_removal_is_blocked_before_mapping_data_can_be_dropped(): void
    {
        $user = $this->createUser();
        $this->readyAttachment((int) $user->id);
        $this->expectException(\LogicException::class);
        $this->app->make(BridgeRemovalListener::class)->beforeUninstall('jiwonpapa-g7mediabooster', true);
    }

    public function test_removal_is_allowed_only_after_remote_deletion_was_accepted(): void
    {
        $user = $this->createUser();
        $this->readyAttachment((int) $user->id);
        DB::table('g7mb_upload_sessions')->where('board_slug', $this->board->slug)
            ->update(['deletion_requested_at' => now()]);
        $this->app->make(BridgeRemovalGuard::class)->assertSafe();
        self::addToAssertionCount(1);
    }

    protected function getTestBoardSlug(): string
    {
        return 'g7mb-core-free';
    }

    protected function runModuleMigrationIfNeeded(): void
    {
        parent::runModuleMigrationIfNeeded();
        if (! Schema::hasTable('g7mb_upload_sessions')) {
            $this->artisan('migrate', ['--path' => dirname(__DIR__, 2).'/database/migrations', '--realpath' => true]);
        }
    }

    protected function setUp(): void
    {
        parent::setUp();
        DB::table('g7mb_upload_sessions')->where('board_slug', $this->board->slug)->delete();
    }

    public function test_stock_user_create_links_ready_media_and_synchronizes_count(): void
    {
        $user = $this->createUser();
        $this->actingAs($user);
        $attachment = $this->readyAttachment((int) $user->id);
        $request = $this->saveRequest([$attachment->id], $user);
        $response = $this->app->make(AttachmentLinkMiddleware::class)->handle($request, function () use ($user): JsonResponse {
            // This is exactly stock user-create: attachment IDs are not passed by its controller.
            $post = $this->newPost((int) $user->id);

            return new JsonResponse(['success' => true, 'data' => ['id' => $post->id]], 201);
        });
        self::assertSame(201, $response->getStatusCode());
        self::assertSame(1, $response->getData(true)['data']['attachments_count']);
        $attachment->refresh();
        self::assertSame((int) $this->board->id, (int) $attachment->board_id);
        self::assertNotNull($attachment->post_id);
        self::assertSame(1, (int) Post::find($attachment->post_id)->attachments_count);
    }

    public function test_foreign_owner_is_rejected_before_post_mutation(): void
    {
        $owner = $this->createUser();
        $actor = $this->createUser();
        $this->actingAs($actor);
        $attachment = $this->readyAttachment((int) $owner->id);
        $this->expectException(ValidationException::class);
        $this->app->make(AttachmentLinkMiddleware::class)->handle($this->saveRequest([$attachment->id], $actor),
            static function (): JsonResponse {
                self::fail('Native post save must not execute');
            });
    }

    public function test_native_save_failure_rolls_back_all_bridge_writes(): void
    {
        $user = $this->createUser();
        $this->actingAs($user);
        $attachment = $this->readyAttachment((int) $user->id);
        $response = $this->app->make(AttachmentLinkMiddleware::class)->handle($this->saveRequest([$attachment->id], $user),
            function () use ($user): JsonResponse {
                $this->newPost((int) $user->id);

                return new JsonResponse(['success' => false], 500);
            });
        self::assertSame(500, $response->getStatusCode());
        self::assertSame(0, Post::where('board_id', $this->board->id)->count());
        self::assertNull($attachment->fresh()->post_id);
    }

    public function test_board_count_limit_rejects_whole_batch(): void
    {
        $user = $this->createUser();
        $this->actingAs($user);
        $this->board->update(['max_file_count' => 1]);
        $one = $this->readyAttachment((int) $user->id, '018f47f0-1111-7111-8111-111111111111');
        $two = $this->readyAttachment((int) $user->id, '018f47f0-2222-7222-8222-222222222222');
        $this->expectException(ValidationException::class);
        $this->app->make(AttachmentLinkMiddleware::class)->handle($this->saveRequest([$one->id, $two->id], $user),
            static function (): JsonResponse {
                self::fail('Over-limit batch must not save a post');
            });
    }

    public function test_native_secret_gate_is_reused_without_reading_file_bytes(): void
    {
        $owner = $this->createUser();
        $other = $this->createUser();
        $this->actingAs($owner);
        $post = $this->newPost((int) $owner->id);
        $post->update(['is_secret' => true]);
        $attachment = $this->readyAttachment((int) $owner->id);
        $attachment->update(['board_id' => $this->board->id, 'post_id' => $post->id]);
        $authorizer = $this->app->make(NativeAttachmentAuthorizer::class);
        $authorizer->authorize(Request::create('/preview'), $this->board->slug, $attachment, 'thumbnail');
        self::addToAssertionCount(1);
        $this->actingAs($other);
        $this->expectException(AccessDeniedHttpException::class);
        $authorizer->authorize(Request::create('/preview'), $this->board->slug, $attachment, 'thumbnail');
    }

    public function test_native_route_specific_middleware_cannot_be_skipped(): void
    {
        $owner = $this->createUser();
        $attachment = $this->readyAttachment((int) $owner->id);
        $post = $this->newPost((int) $owner->id);
        $attachment->update(['board_id' => $this->board->id, 'post_id' => $post->id]);
        $uri = '/api/modules/sirsoft-board/boards/'.$this->board->slug.'/attachment/'.$attachment->hash;
        $request = Request::create($uri);
        $route = new Route(['GET'], 'api/modules/sirsoft-board/boards/{slug}/attachment/{hash}', [
            'uses' => 'Modules\\Sirsoft\\Board\\Http\\Controllers\\User\\AttachmentController@download',
            'middleware' => ['api', DenyMediaRoute::class],
        ]);
        $route->bind($request);
        $route->setContainer($this->app);
        $request->setRouteResolver(static fn () => $route);
        $response = $this->app->make(NativeDeliveryMiddleware::class)->handle($request,
            fn (Request $request) => $this->app->make(Pipeline::class)->send($request)
                ->through([DenyMediaRoute::class])->then(static fn () => $route->run()));
        self::assertSame(403, $response->getStatusCode());
        self::assertIsString($route->getAction('uses'));
    }

    public function test_native_preview_redirects_only_after_authorization_and_still_works_with_uploads_off(): void
    {
        $owner = $this->createUser();
        $this->actingAs($owner);
        $attachment = $this->readyAttachment((int) $owner->id);
        $post = $this->newPost((int) $owner->id);
        $attachment->update(['board_id' => $this->board->id, 'post_id' => $post->id,
            'meta' => ['g7mb_upload_id' => $attachment->path, 'g7mb_preset_id' => 'board-v1',
                'g7mb_thumbnail_content_type' => 'image/jpeg', 'g7mb_thumbnail_size' => 512]]);
        $this->app->instance(MediaBoosterConfiguration::class, MediaBoosterConfiguration::fromArray([
            'enabled' => false, 'hmac_secret' => str_repeat('test-key-', 8),
        ]));
        Http::preventStrayRequests();
        Http::fake(['*' => Http::response([
            'upload_id' => $attachment->path, 'preset_id' => 'board-v1', 'variant' => 'thumbnail',
            'delivery_url' => 'https://storage.example.test/image.jpg?signature=test',
            'expires_at' => gmdate('Y-m-d\TH:i:s\Z', time() + 300), 'content_type' => 'image/jpeg', 'byte_len' => 512,
        ])]);
        $uri = '/api/modules/sirsoft-board/boards/'.$this->board->slug.'/attachment/'.$attachment->hash.'/preview';
        $request = Request::create($uri);
        $route = new Route(['GET'], 'api/modules/sirsoft-board/boards/{slug}/attachment/{hash}/preview', [
            'uses' => 'Modules\\Sirsoft\\Board\\Http\\Controllers\\User\\AttachmentController@preview',
            'middleware' => ['api'],
        ]);
        $route->bind($request);
        $route->setContainer($this->app);
        $request->setRouteResolver(static fn () => $route);
        $response = $this->app->make(NativeDeliveryMiddleware::class)->handle($request,
            static fn () => $route->run());
        self::assertSame(302, $response->getStatusCode(), (string) $response->getContent());
        self::assertSame('https://storage.example.test/image.jpg?signature=test', $response->headers->get('Location'));
        Http::assertSentCount(1);
        self::assertIsString($route->getAction('uses'));
    }

    public function test_blinded_media_is_denied_by_the_native_gate(): void
    {
        $owner = $this->createUser();
        $other = $this->createUser();
        $this->actingAs($owner);
        $post = $this->newPost((int) $owner->id);
        $post->update(['status' => 'blinded']);
        $attachment = $this->readyAttachment((int) $owner->id);
        $attachment->update(['board_id' => $this->board->id, 'post_id' => $post->id]);
        $this->actingAs($other);
        $this->expectException(AccessDeniedHttpException::class);
        $this->app->make(NativeAttachmentAuthorizer::class)->authorize(Request::create('/preview'),
            $this->board->slug, $attachment, 'thumbnail');
    }

    private function saveRequest(array $ids, $user): Request
    {
        $uri = '/api/modules/sirsoft-board/boards/'.$this->board->slug.'/posts';
        $request = Request::create($uri, 'POST', ['attachment_ids' => array_map('intval', $ids)]);
        $route = new Route(['POST'], 'api/modules/sirsoft-board/boards/{slug}/posts', [
            'uses' => 'Modules\\Sirsoft\\Board\\Http\\Controllers\\User\\PostController@store',
        ]);
        $route->bind($request);
        $request->setRouteResolver(static fn () => $route);
        $request->setUserResolver(static fn () => $user);

        return $request;
    }

    private function newPost(int $userId): Post
    {
        return $this->app->make(PostService::class)->createPost($this->board->slug,
            ['user_id' => $userId, 'title' => 'Core-free media', 'content' => 'Stock board host', 'status' => 'published']);
    }

    private function readyAttachment(int $userId, string $uploadId = '018f47f0-4444-7444-8444-444444444444'): Attachment
    {
        $store = new UploadSessionStore;
        $store->recordBatch($userId, $this->board->slug, [[
            'client_ref' => $uploadId, 'original_filename' => 'image.jpg', 'declared_kind' => 'image',
            'content_type_hint' => 'image/jpeg', 'content_length' => 100,
        ]], ['batch_id' => $uploadId, 'uploads' => [[
            'client_ref' => $uploadId, 'upload_id' => $uploadId, 'method' => 'single_put',
        ]]]);
        $id = $store->materializeAttachment($uploadId, $userId, $this->board->slug, fn (): int => (int) $this->app->make(AttachmentRepositoryInterface::class)->create($this->board->slug, [
            'board_id' => 0, 'post_id' => null, 'temp_key' => null, 'created_by' => $userId,
            'original_filename' => 'image.jpg', 'stored_filename' => $uploadId.'.jpg',
            'disk' => 'g7mediabooster', 'path' => $uploadId, 'mime_type' => 'image/jpeg',
            'size' => 100, 'collection' => 'attachments', 'order' => 1,
        ])->id);

        return Attachment::findOrFail($id);
    }
}

final class DenyMediaRoute
{
    public function handle(Request $request, \Closure $next): JsonResponse
    {
        return new JsonResponse(['success' => false], 403);
    }
}
