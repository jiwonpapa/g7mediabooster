<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Middleware;

use Closure;
use Illuminate\Http\Request;
use Modules\Jiwonpapa\G7mediabooster\Http\Controllers\User\AttachmentDeliveryController;
use Modules\Sirsoft\Board\Services\AttachmentService;
use Symfony\Component\HttpFoundation\Response;

/** Swap only the request terminal, preserving every native/third-party middleware. */
final class NativeDeliveryMiddleware
{
    public function __construct(private readonly AttachmentService $attachments) {}

    public function handle(Request $request, Closure $next): Response
    {
        $route = $request->route();
        $controller = $route?->getAction('controller') ?? $route?->getAction('uses');
        $allowed = [
            'Modules\\Sirsoft\\Board\\Http\\Controllers\\User\\AttachmentController@download' => ['master', 'user'],
            'Modules\\Sirsoft\\Board\\Http\\Controllers\\User\\AttachmentController@preview' => ['thumbnail', 'user'],
            'Modules\\Sirsoft\\Board\\Http\\Controllers\\Admin\\AttachmentController@download' => ['master', 'admin'],
        ];
        if (! $request->isMethod('GET') || ! is_string($controller) || ! isset($allowed[$controller])) {
            return $next($request);
        }
        $slug = $route->parameter('slug');
        $hash = $route->parameter('hash');
        if (! is_string($slug) || ! is_string($hash)) {
            return $next($request);
        }
        $attachment = $this->attachments->getByHash($slug, $hash);
        if ($attachment === null || $attachment->disk !== 'g7mediabooster') {
            return $next($request);
        }
        [$variant, $context] = $allowed[$controller];
        $original = $route->getAction();
        // Router has already gathered the native middleware. Keep controller metadata
        // for permission scope resolution, replace only the final callable for this request.
        $route->setAction([...$original, 'uses' => static fn (): Response => app(AttachmentDeliveryController::class)->show($slug, $hash, $variant, $request, $context)]);
        try {
            return $next($request);
        } finally {
            // Also restore on denial/exception and for long-lived Laravel workers.
            $route->setAction($original);
        }
    }
}
