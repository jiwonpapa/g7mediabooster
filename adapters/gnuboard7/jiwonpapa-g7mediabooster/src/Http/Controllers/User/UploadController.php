<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Controllers\User;

use App\Http\Controllers\Api\Base\AuthBaseController;
use Illuminate\Http\JsonResponse;
use Illuminate\Http\RedirectResponse;
use Illuminate\Support\Facades\Log;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Exceptions\MediaBoosterUpstreamException;
use Modules\Jiwonpapa\G7mediabooster\Http\Requests\CompleteMultipartRequest;
use Modules\Jiwonpapa\G7mediabooster\Http\Requests\CreateUploadBatchRequest;
use Modules\Jiwonpapa\G7mediabooster\Http\Requests\PresignPartRequest;
use Modules\Jiwonpapa\G7mediabooster\Services\MediaBoosterClient;
use Modules\Jiwonpapa\G7mediabooster\Services\DerivativeDeliveryValidator;
use Modules\Jiwonpapa\G7mediabooster\Services\UploadSessionStore;
use Modules\Sirsoft\Board\Exceptions\BoardNotFoundException;
use Modules\Sirsoft\Board\Services\BoardService;
use Symfony\Component\HttpKernel\Exception\AccessDeniedHttpException;
use Throwable;
use UnexpectedValueException;

final class UploadController extends AuthBaseController
{
    public function __construct(
        private readonly BoardService $boards,
        private readonly MediaBoosterClient $client,
        private readonly MediaBoosterConfiguration $configuration,
        private readonly UploadSessionStore $sessions,
        private readonly DerivativeDeliveryValidator $deliveryValidator,
    ) {
        parent::__construct();
    }

    public function configuration(string $slug): JsonResponse
    {
        try {
            $board = $this->usableBoard($slug);

            return $this->success('업로더 설정을 조회했습니다.', [
                'enabled' => $this->configuration->enabled,
                'max_files' => min(100, max(1, (int) ($board->max_file_count ?? 1))),
                'max_file_size_bytes' => max(1, (int) ($board->max_file_size ?? 10)) * 1024 * 1024,
                'max_parallel_files' => $this->configuration->maxParallelFiles,
                'max_parallel_parts' => $this->configuration->maxParallelParts,
                'max_part_retries' => $this->configuration->maxPartRetries,
                'status_poll_interval_ms' => $this->configuration->statusPollIntervalMs,
            ]);
        } catch (BoardNotFoundException) {
            return $this->notFound('게시판을 찾을 수 없습니다.');
        } catch (AccessDeniedHttpException) {
            return $this->forbidden('이 게시판은 파일 업로드를 사용하지 않습니다.');
        }
    }

    public function create(CreateUploadBatchRequest $request, string $slug): JsonResponse
    {
        try {
            $board = $this->usableBoard($slug);
            $files = $request->validated('files');
            $maxFiles = min(100, max(1, (int) ($board->max_file_count ?? 1)));
            $maxBytes = max(1, (int) ($board->max_file_size ?? 10)) * 1024 * 1024;
            if (count($files) > $maxFiles) {
                return $this->validationError(['files' => ["이 게시판은 최대 {$maxFiles}개까지 업로드할 수 있습니다."]]);
            }
            foreach ($files as $index => $file) {
                if ((int) $file['content_length'] > $maxBytes) {
                    return $this->validationError([
                        "files.{$index}.content_length" => ['게시판 파일 크기 제한을 초과했습니다.'],
                    ]);
                }
            }

            $controlFiles = array_map(static function (array $file): array {
                unset($file['original_filename']);

                return $file;
            }, $files);
            $created = $this->client->createBatch(['files' => $controlFiles]);
            $this->sessions->recordBatch($this->userId(), $slug, $files, $created);

            return $this->success('직접 업로드 예약을 만들었습니다.', $created, 201);
        } catch (BoardNotFoundException) {
            return $this->notFound('게시판을 찾을 수 없습니다.');
        } catch (AccessDeniedHttpException) {
            return $this->forbidden('이 게시판은 파일 업로드를 사용하지 않습니다.');
        } catch (MediaBoosterUpstreamException $error) {
            return $this->upstreamError($error);
        } catch (Throwable $error) {
            return $this->internalFailure($error, 'batch_create_failed');
        }
    }

    public function presignPart(
        PresignPartRequest $request,
        string $slug,
        string $uploadId,
        int $partNumber,
    ): JsonResponse {
        if (! $this->owns($uploadId, $slug)) {
            return $this->notFound('업로드 세션을 찾을 수 없습니다.');
        }

        try {
            $data = $this->client->presignPart($uploadId, $partNumber, (int) $request->validated('content_length'));

            return $this->success('파트 업로드 주소를 만들었습니다.', $data);
        } catch (MediaBoosterUpstreamException $error) {
            return $this->upstreamError($error);
        } catch (Throwable $error) {
            return $this->internalFailure($error, 'part_presign_failed');
        }
    }

    public function completeMultipart(
        CompleteMultipartRequest $request,
        string $slug,
        string $uploadId,
    ): JsonResponse {
        if (! $this->owns($uploadId, $slug)) {
            return $this->notFound('업로드 세션을 찾을 수 없습니다.');
        }

        try {
            $this->client->completeMultipart($uploadId, $request->validated('parts'));
            $this->sessions->markState($uploadId, 'quarantined');

            return $this->success('멀티파트 업로드를 완료했습니다.');
        } catch (MediaBoosterUpstreamException $error) {
            return $this->upstreamError($error);
        } catch (Throwable $error) {
            return $this->internalFailure($error, 'multipart_complete_failed');
        }
    }

    public function abortMultipart(string $slug, string $uploadId): JsonResponse
    {
        if (! $this->owns($uploadId, $slug)) {
            return $this->notFound('업로드 세션을 찾을 수 없습니다.');
        }

        try {
            $this->client->abortMultipart($uploadId);
            $this->sessions->markState($uploadId, 'aborted');

            return $this->success('멀티파트 업로드를 취소했습니다.');
        } catch (MediaBoosterUpstreamException $error) {
            return $this->upstreamError($error);
        } catch (Throwable $error) {
            return $this->internalFailure($error, 'multipart_abort_failed');
        }
    }

    public function confirmSingle(string $slug, string $uploadId): JsonResponse
    {
        if (! $this->owns($uploadId, $slug)) {
            return $this->notFound('업로드 세션을 찾을 수 없습니다.');
        }

        try {
            $this->client->confirmSingle($uploadId);
            $this->sessions->markState($uploadId, 'quarantined');

            return $this->success('단일 업로드를 확인했습니다.');
        } catch (MediaBoosterUpstreamException $error) {
            return $this->upstreamError($error);
        } catch (Throwable $error) {
            return $this->internalFailure($error, 'single_complete_failed');
        }
    }

    public function status(string $slug, string $uploadId): JsonResponse
    {
        if (! $this->owns($uploadId, $slug)) {
            return $this->notFound('업로드 세션을 찾을 수 없습니다.');
        }

        try {
            $status = $this->client->status($uploadId);
            if (is_string($status['state'] ?? null)) {
                $this->sessions->markState($uploadId, $status['state']);
            }
            if (is_array($status['derivatives'] ?? null)) {
                $status['derivatives'] = array_map(
                    fn (mixed $derivative): mixed => $this->withDeliveryUrl(
                        $derivative,
                        $slug,
                        $uploadId,
                    ),
                    $status['derivatives'],
                );
            }

            return $this->success('업로드 상태를 조회했습니다.', $status);
        } catch (MediaBoosterUpstreamException $error) {
            return $this->upstreamError($error);
        } catch (Throwable $error) {
            return $this->internalFailure($error, 'status_read_failed');
        }
    }

    public function derivative(
        string $slug,
        string $uploadId,
        string $variant,
    ): RedirectResponse|JsonResponse {
        if (! $this->owns($uploadId, $slug)) {
            return $this->notFound('업로드 세션을 찾을 수 없습니다.');
        }

        try {
            $delivery = $this->client->derivativeDelivery($uploadId, $variant);
            $url = $this->deliveryValidator->validate($delivery, $uploadId, $variant);

            return new RedirectResponse($url, 302, [
                'Cache-Control' => 'private, no-store',
                'Referrer-Policy' => 'no-referrer',
                'X-Content-Type-Options' => 'nosniff',
            ]);
        } catch (MediaBoosterUpstreamException $error) {
            return $this->upstreamError($error);
        } catch (UnexpectedValueException) {
            return $this->error('미디어 처리 서버의 전달 응답이 올바르지 않습니다.', 502);
        } catch (Throwable $error) {
            return $this->internalFailure($error, 'derivative_delivery_failed');
        }
    }

    public function delete(string $slug, string $uploadId): JsonResponse
    {
        if (! $this->owns($uploadId, $slug)) {
            return $this->notFound('업로드 세션을 찾을 수 없습니다.');
        }

        try {
            $this->client->deleteUpload($uploadId);
            $this->sessions->markState($uploadId, 'deletion_pending');

            return $this->success('미디어 삭제를 예약했습니다.', statusCode: 202);
        } catch (MediaBoosterUpstreamException $error) {
            return $this->upstreamError($error);
        } catch (Throwable $error) {
            return $this->internalFailure($error, 'upload_delete_failed');
        }
    }

    private function usableBoard(string $slug): object
    {
        $board = $this->boards->getBoardBySlug($slug, checkScope: false);
        if (! $board->use_file_upload) {
            throw new AccessDeniedHttpException('board uploads are disabled');
        }

        return $board;
    }

    private function owns(string $uploadId, string $slug): bool
    {
        return $this->sessions->isOwnedBy($uploadId, $this->userId(), $slug);
    }

    private function withDeliveryUrl(mixed $derivative, string $slug, string $uploadId): mixed
    {
        if (! is_array($derivative)
            || ! is_string($derivative['variant'] ?? null)
            || ! in_array($derivative['variant'], ['master', 'thumbnail'], true)
        ) {
            return $derivative;
        }
        $derivative['delivery_url'] = sprintf(
            '/api/modules/jiwonpapa-g7mediabooster/boards/%s/uploads/%s/derivatives/%s',
            rawurlencode($slug),
            rawurlencode($uploadId),
            $derivative['variant'],
        );

        return $derivative;
    }

    private function userId(): int
    {
        return (int) $this->getCurrentUser()?->getKey();
    }

    private function upstreamError(MediaBoosterUpstreamException $error): JsonResponse
    {
        return $this->error($error->getMessage(), $error->httpStatus, [
            'code' => $error->errorCode,
            'request_id' => $error->requestId,
        ]);
    }

    private function internalFailure(Throwable $error, string $operation): JsonResponse
    {
        Log::warning('G7MediaBooster control operation failed', [
            'operation' => $operation,
            'exception' => $error::class,
        ]);

        return $this->error('미디어 업로드 제어 요청을 처리하지 못했습니다.', 500);
    }
}
