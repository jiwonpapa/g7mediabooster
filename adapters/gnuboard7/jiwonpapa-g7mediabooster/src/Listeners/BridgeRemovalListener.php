<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Listeners;

use App\Contracts\Extension\HookListenerInterface;
use Modules\Jiwonpapa\G7mediabooster\Services\BridgeRemovalGuard;

/** Block removal before G7 can drop this module's mapping tables. */
final class BridgeRemovalListener implements HookListenerInterface
{
    public function __construct(private readonly BridgeRemovalGuard $guard) {}

    public static function getSubscribedHooks(): array
    {
        return ['core.modules.before_uninstall' => ['method' => 'beforeUninstall', 'priority' => 1, 'sync' => true]];
    }

    public function handle(...$args): void {}

    public function beforeUninstall(string $identifier, bool $deleteData = false): void
    {
        if ($identifier === 'jiwonpapa-g7mediabooster') {
            $this->guard->assertSafe();
        }
    }
}
