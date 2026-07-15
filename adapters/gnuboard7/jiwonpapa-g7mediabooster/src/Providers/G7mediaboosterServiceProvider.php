<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Providers;

use App\Extension\BaseModuleServiceProvider;
use App\Services\ModuleSettingsService;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Console\Commands\ReconcileAttachmentRetentionCommand;

final class G7mediaboosterServiceProvider extends BaseModuleServiceProvider
{
    protected string $moduleIdentifier = 'jiwonpapa-g7mediabooster';

    public function register(): void
    {
        parent::register();

        $this->app->bind(
            MediaBoosterConfiguration::class,
            fn ($app): MediaBoosterConfiguration => MediaBoosterConfiguration::fromArray(
                $app->make(ModuleSettingsService::class)->get($this->moduleIdentifier) ?? [],
            ),
        );
    }

    public function boot(): void
    {
        parent::boot();

        if ($this->app->runningInConsole()) {
            $this->commands([ReconcileAttachmentRetentionCommand::class]);
        }
    }
}
