import { Fragment, useState } from 'react';
import Button from '../ds/primitives/Button';
import Icon from '../ds/primitives/Icon';
import { call, errorText } from '../lib/api';
import { backendText, tr } from '../lib/i18n';
import type { Answer } from '../lib/types';
import type { MeetingHit } from '../lib/meetings';
import { clockTime } from '../lib/meetings';
import { Input } from '../ui/Input';

export type AssistantProps = {
  openMail: (id: number) => void;
  openMeeting?: (source: MeetingHit) => void;
};

export default function Assistant({ openMail, openMeeting }: AssistantProps) {
  const [question, setQuestion] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [messages, setMessages] = useState<{ question: string; answer: Answer }[]>([]);
  async function ask(q = question) {
    if (!q.trim() || busy) return;
    setQuestion('');
    setBusy(true);
    setError('');
    try {
      const answer = await call<Answer>('ask_assistant', { question: q });
      setMessages((list) => [...list, { question: q, answer }]);
    } catch (e) {
      setError(errorText(e));
      setQuestion(q);
    } finally {
      setBusy(false);
    }
  }
  const suggestions = [
    tr('Che cosa richiede la mia attenzione?', 'What needs my attention?'),
    tr('Riassumi la proposta del Q4', 'Summarize the Q4 proposal'),
    tr('Quali meeting ho in programma?', 'What meetings are coming up?'),
  ];

  return (
    <section className="assistant-page">
      <div className="page-heading">
        <div>
          <span className="page-kicker">
            {tr('Un piccolo aiuto, quando serve', 'A little help, when you need it')}
          </span>
          <h1>{tr('Il tuo assistente', 'Your assistant')}</h1>
          <p>
            {tr(
              'Trova un dettaglio. Riprendi una conversazione. Pianifica il prossimo passo.',
              'Find a detail. Catch up on a conversation. Plan your next step.',
            )}
          </p>
        </div>
        <span className="assistant-symbol">
          <Icon name="spark" size={28} />
        </span>
      </div>
      <div className="assistant-conversation">
        {!messages.length && (
          <div className="assistant-welcome">
            <Icon name="spark" size={36} />
            <h2>{tr('Che cosa vuoi trovare?', 'What would you like to find?')}</h2>
            <p>
              {tr(
                'Chiedi dei tuoi messaggi, dei prossimi eventi o degli appunti dei meeting.',
                'Ask about your messages, upcoming events or meeting notes.',
              )}
            </p>
            <div>
              {suggestions.map((suggestion) => (
                <Button
                  key={suggestion}
                  variant="secondary"
                  className="suggestion"
                  onClick={() => void ask(suggestion)}
                  disabled={busy}
                >
                  {suggestion}
                  <Icon name="arrow" size={16} />
                </Button>
              ))}
            </div>
          </div>
        )}
        {messages.map((message, i) => (
          <Fragment key={i}>
            <div className="chat-question">{message.question}</div>
            <article className="chat-answer">
              <Icon name="spark" size={20} />
              <div>
                <p>{backendText(message.answer.text)}</p>
                {message.answer.sources.length > 0 && (
                  <div className="answer-sources">
                    {message.answer.sources.map((source, j) => (
                      <Button
                        key={j}
                        variant="secondary"
                        className="source-chip"
                        onClick={() => openMail(source.email_id)}
                      >
                        <Icon name="mail" size={14} />
                        {source.passage.slice(0, 65)}…
                      </Button>
                    ))}
                  </div>
                )}
                {!!message.answer.meeting_sources?.length && (
                  <div className="answer-sources">
                    {message.answer.meeting_sources.map((source, j) => (
                      <Button
                        key={j}
                        variant="secondary"
                        className="source-chip"
                        onClick={() => openMeeting?.(source)}
                      >
                        <Icon name="mic" size={14} />
                        {source.title}
                        {source.at_ms !== null ? ` · ${clockTime(source.at_ms)}` : ''}:{' '}
                        {source.text.slice(0, 65)}…
                      </Button>
                    ))}
                  </div>
                )}
              </div>
            </article>
          </Fragment>
        ))}
        {busy && (
          <p className="muted">
            {tr('Sto leggendo i messaggi pertinenti…', 'Reading the relevant messages…')}
          </p>
        )}
        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
      </div>
      <form
        className="ask-form"
        onSubmit={(e) => {
          e.preventDefault();
          void ask();
        }}
      >
        <Icon name="spark" size={20} />
        <Input
          aria-label={tr('Chiedi al tuo assistente', 'Ask your assistant')}
          value={question}
          onChange={(e) => setQuestion(e.currentTarget.value)}
          placeholder={tr('Chiedi qualcosa sulla tua posta…', 'Ask about your mail…')}
        />
        <Button
          variant="primary"
          type="submit"
          className="ask-send"
          disabled={busy || !question.trim()}
          aria-label={tr('Chiedi all’assistente', 'Ask assistant')}
        >
          <Icon name="arrow" size={18} />
        </Button>
      </form>
    </section>
  );
}
