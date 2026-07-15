<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Controllers\User;

use App\Extension\HookManager;
use App\Http\Controllers\Api\Base\PublicBaseController;
use Illuminate\Http\JsonResponse;
use Illuminate\Http\RedirectResponse;
use Illuminate\Support\Facades\Log;
use LogicException;
use Modules\Jiwonpapa\G7mediabooster\Exceptions\MediaBoosterUpstreamException;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentBridgeService;
use Modules\Jiwonpapa\G7mediabooster\Services\DerivativeDeliveryValidator;
use Modules\Jiwonpapa\G7mediabooster\Services\MediaBoosterClient;
use Modules\Jiwonpapa\G7mediabooster\Services\UploadSessionStore;
use Modules\Sirsoft\Board\Exceptions\BoardNotFoundException;
use Modules\Sirsoft\Board\Services\AttachmentService;
use Modules\Sirsoft\Board\Services\BoardService;
use Symfony\Component\HttpKernel\Exception\AccessDeniedHttpException;
use Throwable;
use UnexpectedValueException;

final class AttachmentDeliveryController extends PublicBaseController
{
    public function __construct(
        private readonly BoardService $boards,
        private readonly AttachmentService $attachments,
        private readonly UploadSessionStore $sessions,
        private readonly MediaBoosterClient $client,
        private readonly DerivativeDeliveryValidator $deliveryValidator,
    ) {
        parent::__construct();
    }

    public function show(string $slug, string $hash, string $variant): RedirectResponse|JsonResponse
    {
        try {
            AttachmentBridgeService::assertSecureUpstreamContract();
            $this->boards->getBoardBySlug($slug, checkScope: false);
            $attachment = $this->attachments->getByHash($slug, $hash);
            if ($attachment === null
                || $attachment->disk !== 'g7mediabooster'
                || ! is_string($attachment->path)
            ) {
                return $this->notFound('첨부파일을 찾을 수 없습니다.');
            }

            $uploadId = strtolower($attachment->path);
            if (! preg_match(
                '/^[a-f0-9]{8}-[a-f0-9]{4}-[1-8][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/',
                $uploadId,
            )) {
                return $this->notFound('첨부파일을 찾을 수 없습니다.');
            }

            $authorized = $this->attachments->authorizeDelivery($slug, (int) $attachment->id, context: 'user');
            if ($authorized === null
                || (int) $authorized->id !== (int) $attachment->id
                || ! $this->sessions->isMaterializedAs($uploadId, (int) $attachment->id, $slug)
            ) {
                return $this->notFound('첨부파일을 찾을 수 없습니다.');
            }

            $delivery = $this->client->derivativeDelivery($uploadId, $variant);
            [$expectedPresetId, $expectedContentType, $expectedByteLen] = $this->expectedDerivative($attachment, $uploadId, $variant);
            $url = $this->deliveryValidator->validateExact(
                $delivery,
                $uploadId,
                $variant,
                $expectedPresetId,
                $expectedContentType,
                $expectedByteLen,
            );
            if ($variant === 'master') {
                HookManager::doAction('sirsoft-board.attachment.after_download', $authorized, 'user');
            }

            return new RedirectResponse($url, 302, [
                'Cache-Control' => 'private, no-store',
                'Referrer-Policy' => 'no-referrer',
                'X-Content-Type-Options' => 'nosniff',
            ]);
        } catch (BoardNotFoundException) {
            return $this->notFound('게시판을 찾을 수 없습니다.');
        } catch (AccessDeniedHttpException) {
            return $this->forbidden('첨부파일을 볼 권한이 없습니다.');
        } catch (MediaBoosterUpstreamException $error) {
            return $this->error($error->getMessage(), $error->httpStatus, [
                'code' => $error->errorCode,
                'request_id' => $error->requestId,
            ]);
        } catch (UnexpectedValueException) {
            return $this->error('미디어 처리 서버의 전달 응답이 올바르지 않습니다.', 502);
        } catch (LogicException) {
            return $this->error('G7 보안 첨부 계약이 설치되지 않았습니다.', 503);
        } catch (Throwable $error) {
            Log::warning('G7MediaBooster attachment delivery failed', [
                'operation' => 'attachment_delivery',
                'exception' => $error::class,
            ]);

            return $this->error('첨부파일 전달을 처리하지 못했습니다.', 500);
        }
    }

    /** @return array{string, string, int} */
    private function expectedDerivative(object $attachment, string $uploadId, string $variant): array
    {
        $meta = $attachment->meta ?? null;
        if (! is_array($meta) || ($meta['g7mb_upload_id'] ?? null) !== $uploadId) {
            throw new UnexpectedValueException('native attachment metadata is invalid');
        }
        $presetId = $meta['g7mb_preset_id'] ?? null;
        if (! is_string($presetId) || ! preg_match('/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/', $presetId)) {
            throw new UnexpectedValueException('native attachment preset is invalid');
        }

        if ($variant === 'master') {
            $size = filter_var($attachment->size ?? null, FILTER_VALIDATE_INT);
            $contentType = $attachment->mime_type ?? null;
        } else {
            $size = filter_var($meta['g7mb_thumbnail_size'] ?? null, FILTER_VALIDATE_INT);
            $contentType = $meta['g7mb_thumbnail_content_type'] ?? null;
        }
        if (! is_int($size)
            || $size < 1
            || ! is_string($contentType)
            || ! in_array($contentType, ['image/jpeg', 'video/mp4'], true)
            || ($variant === 'thumbnail' && $contentType !== 'image/jpeg')
        ) {
            throw new UnexpectedValueException('native derivative metadata is invalid');
        }

        return [$presetId, $contentType, $size];
    }
}
