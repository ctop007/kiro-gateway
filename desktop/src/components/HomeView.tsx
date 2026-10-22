import type { AppState } from '@/state/useAppState'
import type { TabId } from '@/features/shell/tabs'
import { SwitchPanel } from './SwitchPanel'
import { KclaudePanel } from './KclaudePanel'
import { ChecksPanel } from './ChecksPanel'
import { UsageStrip } from './UsageStrip'

export function HomeView({ app, goto }: { app: AppState; goto: (t: TabId) => void }) {
  return (
    <div className="home">
      <div className="col">
        <SwitchPanel app={app} onChangePort={() => goto('settings')} />
        <KclaudePanel app={app} />
      </div>
      <div className="col">
        <ChecksPanel app={app} />
        <UsageStrip app={app} goto={goto} />
      </div>
    </div>
  )
}
