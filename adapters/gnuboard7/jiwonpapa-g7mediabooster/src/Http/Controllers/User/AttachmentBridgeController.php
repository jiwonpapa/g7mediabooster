<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Controllers\User;

use App\Http\Controllers\Api\Base\AuthBaseController;
use Illuminate\Http\JsonResponse;
use Illuminate\Support\Facades\Log;
use LogicException;
use Modules\Jiwonpapa\G7mediabooster\Exceptions\MediaBoosterUpstreamException;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentBridgeService;
use Modules\Sirsoft\Board\Exceptions\BoardNotFoundException;
use Modules\Sirsoft\Board\Services\BoardService;
use Throwable;
use UnexpectedValueException;

final class AttachmentBridgeController extends AuthBaseController
{
    public function __construct(
        private readonly BoardService $boards,
        private readonly AttachmentBridgeService $bridge,
    ) {
        parent::__construct();
    }

    public function store(string $slug, string $uploadId): JsonResponse
    {
        try {
            $board = $this->boards->getBoardBySlug($slug, checkScope: false);
            if (! $board->use_file_upload) {
                return $this->forbidden('이 게시판은 파일 업로드를 사용하지 않습니다.');
            }

            $attachment = $this->bridge->materialize(
                $uploadId,
                (int) $this->getCurrentUser()?->getKey(),
                $slug,
            );

            return $this->success('안전 검사가 끝난 미디어를 첨부파일로 준비했습니다.', [
                'data' => [
                    'id' => $attachment->id,
                    'hash' => $attachment->hash,
                    'original_filename' => $attachment->original_filename,
                    'stored_filename' => $attachment->stored_filename,
                    'mime_type' => $attachment->mime_type,
                    'size' => $attachment->size,
                    'url' => $this->deliveryPath($slug, (string) $attachment->hash, 'master'),
                    'preview_url' => $this->deliveryPath($slug, (string) $attachment->hash, 'thumbnail'),
                    'order' => $attachment->order,
                    'created_at' => $attachment->created_at,
                ],
            ], 201);
        } catch (BoardNotFoundException) {
            return $this->notFound('게시판을 찾을 수 없습니다.');
        } catch (MediaBoosterUpstreamException $error) {
            return $this->error($error->getMessage(), $error->httpStatus, [
                'code' => $error->errorCode,
                'request_id' => $error->requestId,
            ]);
        } catch (UnexpectedValueException) {
            return $this->error('미디어가 아직 안전한 첨부파일로 준비되지 않았습니다.', 409);
        } catch (LogicException) {
            return $this->error('G7 보안 첨부 계약이 설치되지 않았습니다.', 503);
        } catch (Throwable $error) {
            Log::warning('G7MediaBooster attachment materialization failed', [
                'operation' => 'attachment_materialize',
                'exception' => $error::class,
            ]);

            return $this->error('첨부파일 연결을 처리하지 못했습니다.', 500);
        }
    }

    private function deliveryPath(string $slug, string $hash, string $variant): string
    {
        return sprintf(
            '/api/modules/jiwonpapa-g7mediabooster/boards/%s/attachments/%s/%s',
            $slug,
            $hash,
            $variant,
        );
    }
}
