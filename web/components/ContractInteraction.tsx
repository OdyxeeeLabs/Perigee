import React, { useEffect, useState } from 'react';
import { useTranslations } from 'next-intl';
import type { ContractFunction, SimulationInputs } from '../lib/sorobantypes';
import { useAnnouncer } from './AnnouncerProvider';
import { DynamicForm } from './DynamicForm';

interface ContractInteractionProps {
  selectedFunction: ContractFunction;
  loading?: boolean;
  isLoading?: boolean;
  onSubmit?: (inputs: SimulationInputs) => Promise<void> | void;
  onSimulate?: (inputs: SimulationInputs, customWasmData?: string) => Promise<void> | void;
  contractId?: string;
  onContractIdChange?: (value: string) => void;
}

export const ContractInteraction: React.FC<ContractInteractionProps> = ({
  selectedFunction,
  loading,
  isLoading,
  onSubmit,
  onSimulate,
}) => {
  const t = useTranslations();
  const { announceStatus, announceAlert } = useAnnouncer();
  const busy = loading ?? isLoading ?? false;
  const [outcome, setOutcome] = useState<'idle' | 'success' | 'error'>('idle');

  useEffect(() => {
    if (busy) setOutcome('idle');
  }, [busy]);

  useEffect(() => {
    if (outcome === 'success') {
      announceStatus(t('a11y.resultReady', { outcome: t('result.success') }));
    } else if (outcome === 'error') {
      announceAlert(`${t('a11y.analysisFailed')} ${t('result.error')}`.trim());
    }
  }, [outcome, announceStatus, announceAlert, t]);

  const handleSubmit = async (inputs: SimulationInputs) => {
    const submit = onSubmit ?? onSimulate;
    if (!submit) return;
    try {
      await submit(inputs);
      setOutcome('success');
    } catch {
      setOutcome('error');
    }
  };

  return (
    <div
      style={{
        backgroundColor: '#161b22',
        borderRadius: '8px',
        padding: '24px',
        border: '1px solid #30363d',
      }}
    >
      <h2
        style={{
          margin: '0 0 16px 0',
          fontSize: '18px',
          fontWeight: '600',
          color: '#58a6ff',
        }}
      >
        {selectedFunction.name}
      </h2>
      <DynamicForm
        func={selectedFunction}
        onSubmit={handleSubmit}
        loading={busy}
      />
    </div>
  );
};
