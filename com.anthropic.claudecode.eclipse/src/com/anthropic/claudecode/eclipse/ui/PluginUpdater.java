package com.anthropic.claudecode.eclipse.ui;

import java.net.URI;
import java.util.Collection;
import java.util.Set;
import java.util.function.Consumer;

import org.eclipse.core.runtime.IProgressMonitor;
import org.eclipse.core.runtime.IStatus;
import org.eclipse.core.runtime.Status;
import org.eclipse.core.runtime.jobs.Job;
import org.eclipse.equinox.p2.core.IProvisioningAgent;
import org.eclipse.equinox.p2.engine.IProfile;
import org.eclipse.equinox.p2.engine.IProfileRegistry;
import org.eclipse.equinox.p2.metadata.IInstallableUnit;
import org.eclipse.equinox.p2.operations.UpdateOperation;
import org.eclipse.equinox.p2.query.QueryUtil;
import org.eclipse.equinox.p2.repository.artifact.IArtifactRepositoryManager;
import org.eclipse.equinox.p2.repository.metadata.IMetadataRepositoryManager;
import org.eclipse.equinox.p2.ui.ProvisioningUI;
import org.eclipse.swt.widgets.Display;

import com.google.gson.JsonObject;

/**
 * Updates this plug-in alone, through Eclipse's own update wizard. Not "Help ▸ Check for
 * Updates", which looks through every update site the Eclipse knows of and offers whatever it
 * finds there: this asks only this plug-in's feature, of only this plug-in's site, and the
 * wizard that opens lists that one thing.
 *
 * <p>Reads the site afresh each time ({@link #refresh}).
 *
 * <p>Links the p2 bundles, which the plug-in requires as optional: an Eclipse without them
 * fails on first use of this class, with an {@link Error} that {@link #start} turns into
 * "unavailable" for the caller to fall back on.
 */
final class PluginUpdater {
    private PluginUpdater() {}

    /** The feature this plug-in is installed as. */
    private static final String FEATURE = "com.anthropic.claudecode.eclipse.feature.feature.group";

    /** The update site the README installs it from. */
    private static final String SITE =
            "https://eilonwy06.github.io/claudecode-eclipse-ide/com.anthropic.claudecode.eclipse.site/";

    /**
     * Looks on a background job for a newer version of the feature on the site and, when there
     * is one, opens the update wizard. Answers {@code cb} (on any thread) with
     * {@code {"status": …, "message": …}}: {@code wizard} (opened), {@code nothing} (the site has
     * nothing newer than what is installed), {@code notinstalled} (this copy was not installed
     * from a site, as one run from a workspace is not), {@code error}, or {@code unavailable}
     * (this Eclipse cannot do it).
     */
    static void start(Consumer<String> cb) {
        try {
            ProvisioningUI ui = ProvisioningUI.getDefaultUI();
            Job job = new Job("Looking for a newer Claude Code plug-in") {
                @Override protected IStatus run(IProgressMonitor monitor) {
                    try {
                        cb.accept(look(ui, monitor));
                    } catch (LinkageError e) {   // a p2 bundle this needs is not there
                        cb.accept(answer("unavailable", String.valueOf(e.getMessage())));
                    } catch (Throwable t) {
                        cb.accept(answer("error", String.valueOf(t.getMessage())));
                    }
                    return Status.OK_STATUS;
                }
            };
            job.setUser(false);
            job.schedule();
        } catch (Throwable t) {
            cb.accept(answer("unavailable", String.valueOf(t)));
        }
    }

    private static String look(ProvisioningUI ui, IProgressMonitor monitor) throws Exception {
        IProvisioningAgent agent = ui.getSession().getProvisioningAgent();
        IProfileRegistry profiles = agent.getService(IProfileRegistry.class);
        IProfile profile = profiles == null ? null : profiles.getProfile(ui.getProfileId());
        if (profile == null) return answer("notinstalled", "");
        Set<IInstallableUnit> installed = profile.query(QueryUtil.createIUQuery(FEATURE), monitor).toUnmodifiableSet();
        if (installed.isEmpty()) return answer("notinstalled", "");
        refresh(agent, monitor);
        UpdateOperation op = ui.getUpdateOperation((Collection<IInstallableUnit>) installed, new URI[] { URI.create(SITE) });
        IStatus found = op.resolveModal(monitor);
        if (found.getCode() == UpdateOperation.STATUS_NOTHING_TO_UPDATE) return answer("nothing", "");
        if (found.getSeverity() == IStatus.ERROR || found.getSeverity() == IStatus.CANCEL) {
            return answer("error", found.getMessage());
        }
        Display.getDefault().asyncExec(() -> ui.openUpdateWizard(false, op, null));
        return answer("wizard", "");
    }

    /**
     * Has p2 read the site again. It keeps a repository it has loaded for the whole session, and
     * a copy of its metadata on disk, so without this a release published after Eclipse first
     * looked at the site would not be seen until Eclipse was restarted. A site p2 has not loaded
     * is read afresh anyway, and a refresh that fails leaves the lookup to the cached copy.
     */
    private static void refresh(IProvisioningAgent agent, IProgressMonitor monitor) {
        URI site = URI.create(SITE);
        try {
            IMetadataRepositoryManager metadata = agent.getService(IMetadataRepositoryManager.class);
            if (metadata != null && metadata.contains(site)) metadata.refreshRepository(site, monitor);
        } catch (Exception ignored) {}
        try {
            IArtifactRepositoryManager artifacts = agent.getService(IArtifactRepositoryManager.class);
            if (artifacts != null && artifacts.contains(site)) artifacts.refreshRepository(site, monitor);
        } catch (Exception ignored) {}
    }

    private static String answer(String status, String message) {
        JsonObject o = new JsonObject();
        o.addProperty("status", status);
        o.addProperty("message", message);
        return o.toString();
    }
}
