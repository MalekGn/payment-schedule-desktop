<script setup lang="ts">
import { reactive, ref } from "vue";
import { useI18n } from "vue-i18n";
import BaseModal from "@/components/ui/BaseModal.vue";
import { useUiStore } from "@/stores/ui";
import { api } from "@/api";
import type { Client } from "@/types/models";

const props = defineProps<{ client?: Client | null }>();
const emit = defineEmits<{ close: []; saved: [client: Client] }>();

const { t } = useI18n();
const ui = useUiStore();

const form = reactive({
  firstName: props.client?.firstName ?? "",
  lastName: props.client?.lastName ?? "",
  phone: props.client?.phone ?? "",
  address: props.client?.address ?? "",
  email: props.client?.email ?? "",
});
const errors = reactive<Record<string, string>>({});
const saving = ref(false);

function validate(): boolean {
  for (const k of Object.keys(errors)) delete errors[k];
  if (!form.firstName.trim()) errors.firstName = t("validation.required");
  if (!form.lastName.trim()) errors.lastName = t("validation.required");
  if (form.email.trim() && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(form.email.trim())) {
    errors.email = t("validation.invalidEmail");
  }
  return Object.keys(errors).length === 0;
}

async function submit() {
  if (!validate()) return;
  saving.value = true;
  try {
    const payload = {
      firstName: form.firstName,
      lastName: form.lastName,
      phone: form.phone,
      address: form.address,
      email: form.email.trim() || null,
    };
    const saved = props.client
      ? await api.updateClient(props.client.id, payload)
      : await api.createClient(payload);
    ui.notify(t("common.save"));
    emit("saved", saved);
  } catch (e) {
    ui.notify(String(e), "error");
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <BaseModal
    :title="client ? t('clients.form.editTitle') : t('clients.form.newTitle')"
    @close="emit('close')"
  >
    <form class="client-form" @submit.prevent="submit">
      <div class="grid-2">
        <div class="field">
          <label for="cf-first">{{ t("clients.form.firstName") }}</label>
          <input
            id="cf-first"
            v-model="form.firstName"
            class="input"
            :class="{ 'input--error': errors.firstName }"
          />
          <span v-if="errors.firstName" class="field-error">{{ errors.firstName }}</span>
        </div>
        <div class="field">
          <label for="cf-last">{{ t("clients.form.lastName") }}</label>
          <input
            id="cf-last"
            v-model="form.lastName"
            class="input"
            :class="{ 'input--error': errors.lastName }"
          />
          <span v-if="errors.lastName" class="field-error">{{ errors.lastName }}</span>
        </div>
      </div>
      <div class="field">
        <label for="cf-phone">{{ t("clients.form.phone") }}</label>
        <input
          id="cf-phone"
          v-model="form.phone"
          class="input"
          :placeholder="t('clients.form.phonePlaceholder')"
          inputmode="tel"
        />
      </div>
      <div class="field">
        <label for="cf-addr">{{ t("clients.form.address") }}</label>
        <input id="cf-addr" v-model="form.address" class="input" />
      </div>
      <div class="field">
        <label for="cf-email">
          {{ t("clients.form.email") }} <span class="muted">({{ t("common.optional") }})</span>
        </label>
        <input
          id="cf-email"
          v-model="form.email"
          class="input"
          :class="{ 'input--error': errors.email }"
          type="email"
        />
        <span v-if="errors.email" class="field-error">{{ errors.email }}</span>
      </div>
    </form>

    <template #footer>
      <button class="btn btn--ghost" type="button" @click="emit('close')">
        {{ t("common.cancel") }}
      </button>
      <button class="btn btn--primary" type="button" :disabled="saving" @click="submit">
        {{ client ? t("common.saveChanges") : t("common.create") }}
      </button>
    </template>
  </BaseModal>
</template>

<style scoped>
.client-form {
  display: flex;
  flex-direction: column;
  gap: 14px;
}
.grid-2 {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 14px;
}
</style>
