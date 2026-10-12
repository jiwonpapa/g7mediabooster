<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Middleware;

use Closure;
use Illuminate\Http\JsonResponse;
use Illuminate\Http\Request;
use Illuminate\Support\Facades\DB;
use Illuminate\Validation\ValidationException;
use Modules\Sirsoft\Board\Models\Attachment;
use Modules\Sirsoft\Board\Models\Board;
use Modules\Sirsoft\Board\Repositories\Contracts\AttachmentRepositoryInterface;
use Modules\Sirsoft\Board\Repositories\Contracts\PostRepositoryInterface;
use Symfony\Component\HttpFoundation\Response;
use Throwable;

/** Atomic owner-scoped bridge around the native post save; never patches PostService. */
final class AttachmentLinkMiddleware
{
    public function __construct(private readonly AttachmentRepositoryInterface $attachments, private readonly PostRepositoryInterface $posts) {}

    public function handle(Request $request, Closure $next): Response
    {
        $route = $request->route();
        $controller = $route?->getAction('controller') ?? $route?->getAction('uses');
        if (! is_string($controller) || ! preg_match(
            '#^Modules\\\\Sirsoft\\\\Board\\\\Http\\\\Controllers\\\\(?:User|Admin)\\\\PostController@(store|update)$#', $controller,
        ) || ! $request->has('attachment_ids')) {
            return $next($request);
        }
        $ids = $request->input('attachment_ids');
        if (! is_array($ids) || ! array_is_list($ids) || count($ids) > 100
            || count(array_unique($ids, SORT_REGULAR)) !== count($ids)
            || array_filter($ids, static fn ($id): bool => ! is_int($id) || $id < 1) !== []) {
            throw ValidationException::withMessages(['attachment_ids' => '유효한 첨부 ID를 최대 100개까지 중복 없이 지정하세요.']);
        }
        if ($ids === [] || ! Attachment::query()->whereIn('id', $ids)->where('disk', 'g7mediabooster')->exists()) {
            return $next($request);
        }
        DB::beginTransaction();
        try {
            $slug = (string) $route->parameter('slug');
            $board = Board::query()->where('slug', $slug)->lockForUpdate()->firstOrFail();
            $actor = $request->user('sanctum') ?? $request->user();
            $postId = (int) $route->parameter('id', 0);
            $rows = Attachment::query()->whereIn('id', $ids)->lockForUpdate()->get();
            $pending = [];
            if ($rows->count() !== count($ids) || $actor === null) {
                $this->reject();
            }
            foreach ($rows as $attachment) {
                if ($attachment->post_id !== null) {
                    if ($postId < 1 || (int) $attachment->post_id !== $postId || (int) $attachment->board_id !== (int) $board->id) {
                        $this->reject();
                    }

                    continue;
                }
                if ((int) $attachment->created_by !== (int) $actor->getAuthIdentifier()) {
                    $this->reject();
                }
                if ($attachment->disk === 'g7mediabooster') {
                    $session = DB::table('g7mb_upload_sessions')->where('attachment_id', $attachment->id)
                        ->where('upload_id', $attachment->path)->where('board_slug', $slug)
                        ->where('user_id', $actor->getAuthIdentifier())->where('state', 'ready')
                        ->whereNotNull('materialized_at')->where('ownership_expires_at', '>', now())
                        ->whereNull('retention_request_started_at')->whereNull('deletion_requested_at')->lockForUpdate()->first();
                    if ($session === null) {
                        $this->reject();
                    }
                }
                $pending[] = (int) $attachment->id;
            }
            $existing = $postId > 0 ? Attachment::query()->where('board_id', $board->id)->where('post_id', $postId)->count() : 0;
            if (! $board->use_file_upload || ((int) $board->max_file_count > 0 && $existing + count($pending) > (int) $board->max_file_count)) {
                $this->reject();
            }
            // Native route authentication, permission, FormRequest and PostService all run.
            $response = $next($request);
            if ($response->getStatusCode() >= 400) {
                DB::rollBack();

                return $response;
            }
            $body = $response instanceof JsonResponse ? $response->getData(true) : [];
            $postId = $postId ?: (int) ($body['data']['id'] ?? 0);
            if ($postId < 1) {
                $this->reject();
            }
            // Stock 7.1 user-create omits attachment_ids. Finish the same repository link
            // inside our outer transaction; update/admin paths are idempotent here.
            $this->attachments->linkAttachmentsByIds($slug, $pending, $postId);
            $linked = Attachment::query()->where('board_id', $board->id)->where('post_id', $postId)->whereIn('id', $ids)->count();
            if ($linked !== count($ids)) {
                $this->reject();
            }
            $count = $this->posts->recalculateAttachmentsCount($postId);
            if ($response instanceof JsonResponse && isset($body['data']['id'])) {
                $body['data']['attachments_count'] = $count;
                $response->setData($body);
            }
            DB::commit();

            return $response;
        } catch (Throwable $error) {
            DB::rollBack();
            throw $error;
        }
    }

    private function reject(): never
    {
        throw ValidationException::withMessages(['attachment_ids' => '첨부 소유권·게시판·처리 상태·개수를 확인할 수 없습니다.']);
    }
}
