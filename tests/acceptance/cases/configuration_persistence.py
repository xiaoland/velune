"""Save and edit configuration and unfinished input through generated bindings.

Both reopenings construct a new application owner over the files written by
the previous owner. No Harness, network response or prewritten state is used.
"""
from __future__ import annotations

from velune_acceptance import Context

REQUIRES = ()
PROVIDER = "acceptance-offline"
INPUT_ID = "cda5b1ae-34e5-4ab3-89c3-4781a947547e"
API_KEY = "synthetic-acceptance-key"
ATTACHMENT = b"\x00\x7f\x80\xff\r\nVelune attachment\x00"


def run(ctx: Context) -> None:
    bindings, api = ctx.bindings.bindings, ctx.bindings.application_api
    options = bindings.BindingOptions(home_directory=str(ctx.isolated.state),
                                      resources_directory=str(ctx.isolated.resources))
    app = bindings.VeluneApplication.open(options)

    def provider(name, window, record_key=""):
        return api.ProviderDraft(
            id=PROVIDER, name=name, protocol=api.Protocol.MESSAGES,
            endpoint="http://127.0.0.1:1/acceptance", execution_backend=api.ExecutionBackend.HTTP(),
            request_profile=api.RequestProfile.NATIVE,
            authentication_source=api.ProviderAuthenticationSource.API_KEY,
            quota_source=api.ProviderQuotaSource.NONE, quota_limits=[], codex_client_version=None,
            models=[api.ProviderModel(record_key=record_key, provider_model_id="synthetic-model",
                                      context_window=window, max_output_tokens=4096)])

    def saved_provider():
        return next(item for gateway in app.list().gateways if gateway.id == "default"
                    for item in gateway.providers if item.id == PROVIDER)

    try:
        app.save_provider("default", provider("Initial synthetic provider", 32000),
                          api.AuthenticationEdit.SET_API_KEY(API_KEY))
        app.shutdown()
        app = bindings.VeluneApplication.open(options)
        first = saved_provider()
        assert first.name == "Initial synthetic provider"
        assert first.endpoint == "http://127.0.0.1:1/acceptance" and first.protocol == api.Protocol.MESSAGES
        assert len(first.models) == 1 and first.models[0].provider_model_id == "synthetic-model"
        assert first.models[0].record_key, "saved model is missing its identity"
        assert first.models[0].context_window == 32000 and first.models[0].max_output_tokens == 4096
        assert app.read_provider_api_key("default", PROVIDER) == API_KEY
        ctx.passed("saved_provider_and_synthetic_key_survive_reopen")

        app.save_provider("default", provider("Edited synthetic provider", 64000, first.models[0].record_key),
                          api.AuthenticationEdit.KEEP())
        attachment = app.upload_composer_attachment("unfinished.bin", ATTACHMENT)
        assert attachment.name == "unfinished.bin" and attachment.bytes == len(ATTACHMENT)
        app.manage_composer_inputs(api.ComposerInputOperation.SAVE(
            input_id=INPUT_ID, expected_revision=None,
            content=api.ComposerInputContent(text="Unsent text — 未发送", attachments=[attachment])))
        app.shutdown()
        app = bindings.VeluneApplication.open(options)

        edited = saved_provider()
        assert edited.name == "Edited synthetic provider"
        assert len(edited.models) == 1 and edited.models[0].context_window == 64000
        assert edited.models[0].provider_model_id == "synthetic-model"
        assert edited.models[0].record_key == first.models[0].record_key
        assert app.read_provider_api_key("default", PROVIDER) == API_KEY
        ctx.passed("provider_edit_preserves_model_identity_and_kept_key_after_reopen")

        state = app.manage_composer_inputs(api.ComposerInputOperation.READ(since_revision=None)).state
        draft = next(item for item in state.inputs if item.id == INPUT_ID)
        assert draft.content.text == "Unsent text — 未发送" and len(draft.content.attachments) == 1
        restored = draft.content.attachments[0]
        assert restored.id == attachment.id and restored.name == "unfinished.bin"
        assert restored.bytes == len(ATTACHMENT)
        content = app.read_composer_attachment(restored.id)
        assert content.attachment.id == restored.id and bytes(content.bytes) == ATTACHMENT
        ctx.passed("unfinished_text_and_exact_attachment_bytes_survive_reopen")
    finally:
        app.shutdown_stopping_work()
